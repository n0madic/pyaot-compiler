//! Runtime function call code generation
//!
//! This module handles code generation for all RuntimeCall instructions,
//! including print functions, string operations, list operations, tuple
//! operations, dictionary operations, and type conversions.

mod boxing;
mod bytes;
mod cells;
mod class_attrs;
mod compare;
mod conversions;
mod dict;
mod file;
mod generator;
mod globals;
mod hash;
mod instance;
mod iterator;
mod list;
mod math;
mod minmax;
mod object;
mod print;
mod set;
mod stdlib;
mod string;
mod tuple;

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
        | mir::RuntimeFunc::StrFind
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
        | mir::RuntimeFunc::StrRfind
        | mir::RuntimeFunc::StrRindex
        | mir::RuntimeFunc::StrIndex
        | mir::RuntimeFunc::StrRsplit
        | mir::RuntimeFunc::StrIsAscii
        | mir::RuntimeFunc::StrEncode
        // StringBuilder for efficient string concatenation
        | mir::RuntimeFunc::MakeStringBuilder
        | mir::RuntimeFunc::StringBuilderAppend
        | mir::RuntimeFunc::StringBuilderToStr => {
            string::compile_string_call(builder, dest, func, args, ctx)?;
            Ok(())
        }

        // List operations
        mir::RuntimeFunc::MakeList
        | mir::RuntimeFunc::ListPush
        | mir::RuntimeFunc::ListSet
        | mir::RuntimeFunc::ListGet
        | mir::RuntimeFunc::ListGetInt
        | mir::RuntimeFunc::ListGetFloat
        | mir::RuntimeFunc::ListGetBool
        | mir::RuntimeFunc::ListLen
        | mir::RuntimeFunc::ListSlice
        | mir::RuntimeFunc::ListSliceStep
        | mir::RuntimeFunc::ListAppend
        | mir::RuntimeFunc::ListSetElemTag
        | mir::RuntimeFunc::ListPop
        | mir::RuntimeFunc::ListInsert
        | mir::RuntimeFunc::ListRemove
        | mir::RuntimeFunc::ListClear
        | mir::RuntimeFunc::ListIndex
        | mir::RuntimeFunc::ListCount
        | mir::RuntimeFunc::ListCopy
        | mir::RuntimeFunc::ListReverse
        | mir::RuntimeFunc::ListExtend
        | mir::RuntimeFunc::ListSort
        | mir::RuntimeFunc::ListSortWithKey
        | mir::RuntimeFunc::ListFromTuple
        | mir::RuntimeFunc::ListFromStr
        | mir::RuntimeFunc::ListFromRange
        | mir::RuntimeFunc::ListFromIter
        | mir::RuntimeFunc::ListFromSet
        | mir::RuntimeFunc::ListFromDict
        | mir::RuntimeFunc::ListTailToTuple
        | mir::RuntimeFunc::ListTailToTupleFloat
        | mir::RuntimeFunc::ListTailToTupleBool
        | mir::RuntimeFunc::ListSliceAssign
        | mir::RuntimeFunc::ListConcat => {
            list::compile_list_call(builder, dest, func, args, ctx)?;
            Ok(())
        }

        // Tuple operations
        mir::RuntimeFunc::MakeTuple
        | mir::RuntimeFunc::TupleSet
        | mir::RuntimeFunc::TupleGet
        | mir::RuntimeFunc::TupleLen
        | mir::RuntimeFunc::TupleSlice
        | mir::RuntimeFunc::TupleSliceStep
        | mir::RuntimeFunc::TupleSliceToList
        | mir::RuntimeFunc::TupleGetInt
        | mir::RuntimeFunc::TupleGetFloat
        | mir::RuntimeFunc::TupleGetBool
        | mir::RuntimeFunc::TupleFromList
        | mir::RuntimeFunc::TupleFromStr
        | mir::RuntimeFunc::TupleFromRange
        | mir::RuntimeFunc::TupleFromIter
        | mir::RuntimeFunc::TupleFromSet
        | mir::RuntimeFunc::TupleFromDict
        | mir::RuntimeFunc::TupleConcat
        | mir::RuntimeFunc::TupleIndex
        | mir::RuntimeFunc::TupleCount
        | mir::RuntimeFunc::TupleSetHeapMask
        | mir::RuntimeFunc::CallWithTupleArgs => {
            tuple::compile_tuple_call(builder, dest, func, args, ctx)?;
            Ok(())
        }

        // Dict operations
        mir::RuntimeFunc::MakeDict
        | mir::RuntimeFunc::DictSet
        | mir::RuntimeFunc::DictGet
        | mir::RuntimeFunc::DictLen
        | mir::RuntimeFunc::DictContains
        | mir::RuntimeFunc::DictGetDefault
        | mir::RuntimeFunc::DictPop
        | mir::RuntimeFunc::DictClear
        | mir::RuntimeFunc::DictCopy
        | mir::RuntimeFunc::DictKeys
        | mir::RuntimeFunc::DictValues
        | mir::RuntimeFunc::DictItems
        | mir::RuntimeFunc::DictUpdate
        | mir::RuntimeFunc::DictFromPairs
        | mir::RuntimeFunc::DictSetDefault
        | mir::RuntimeFunc::DictPopItem
        | mir::RuntimeFunc::DictFromKeys
        | mir::RuntimeFunc::DictMerge
        | mir::RuntimeFunc::MakeDefaultDict
        | mir::RuntimeFunc::DefaultDictGet
        | mir::RuntimeFunc::MakeCounterFromIter
        | mir::RuntimeFunc::MakeCounterEmpty
        | mir::RuntimeFunc::MakeDeque
        | mir::RuntimeFunc::MakeDequeFromIter => {
            dict::compile_dict_call(builder, dest, func, args, ctx)?;
            Ok(())
        }

        // Boxing/Unboxing operations
        mir::RuntimeFunc::BoxInt
        | mir::RuntimeFunc::BoxBool
        | mir::RuntimeFunc::BoxFloat
        | mir::RuntimeFunc::BoxNone
        | mir::RuntimeFunc::UnboxFloat
        | mir::RuntimeFunc::UnboxInt
        | mir::RuntimeFunc::UnboxBool => {
            boxing::compile_boxing_call(builder, dest, func, args, ctx)?;
            Ok(())
        }

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

        // Hash operations
        mir::RuntimeFunc::HashInt
        | mir::RuntimeFunc::HashStr
        | mir::RuntimeFunc::HashBool
        | mir::RuntimeFunc::HashTuple
        | mir::RuntimeFunc::IdObj => {
            hash::compile_hash_call(builder, dest, func, args, ctx)?;
            Ok(())
        }

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

        // Set operations
        mir::RuntimeFunc::MakeSet
        | mir::RuntimeFunc::SetAdd
        | mir::RuntimeFunc::SetContains
        | mir::RuntimeFunc::SetRemove
        | mir::RuntimeFunc::SetDiscard
        | mir::RuntimeFunc::SetLen
        | mir::RuntimeFunc::SetClear
        | mir::RuntimeFunc::SetCopy
        | mir::RuntimeFunc::SetToList
        | mir::RuntimeFunc::SetUnion
        | mir::RuntimeFunc::SetIntersection
        | mir::RuntimeFunc::SetDifference
        | mir::RuntimeFunc::SetSymmetricDifference
        | mir::RuntimeFunc::SetIssubset
        | mir::RuntimeFunc::SetIssuperset
        | mir::RuntimeFunc::SetIsdisjoint
        | mir::RuntimeFunc::SetPop
        | mir::RuntimeFunc::SetUpdate
        | mir::RuntimeFunc::SetIntersectionUpdate
        | mir::RuntimeFunc::SetDifferenceUpdate
        | mir::RuntimeFunc::SetSymmetricDifferenceUpdate => {
            set::compile_set_call(builder, dest, func, args, ctx)?;
            Ok(())
        }

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

        // Bytes operations
        mir::RuntimeFunc::MakeBytes
        | mir::RuntimeFunc::MakeBytesZero
        | mir::RuntimeFunc::MakeBytesFromList
        | mir::RuntimeFunc::MakeBytesFromStr
        | mir::RuntimeFunc::BytesGet
        | mir::RuntimeFunc::BytesLen
        | mir::RuntimeFunc::BytesSlice
        | mir::RuntimeFunc::BytesSliceStep
        | mir::RuntimeFunc::BytesDecode
        | mir::RuntimeFunc::BytesStartsWith
        | mir::RuntimeFunc::BytesEndsWith
        | mir::RuntimeFunc::BytesFind
        | mir::RuntimeFunc::BytesRfind
        | mir::RuntimeFunc::BytesIndex
        | mir::RuntimeFunc::BytesRindex
        | mir::RuntimeFunc::BytesCount
        | mir::RuntimeFunc::BytesReplace
        | mir::RuntimeFunc::BytesSplit
        | mir::RuntimeFunc::BytesRsplit
        | mir::RuntimeFunc::BytesJoin
        | mir::RuntimeFunc::BytesStrip
        | mir::RuntimeFunc::BytesLstrip
        | mir::RuntimeFunc::BytesRstrip
        | mir::RuntimeFunc::BytesUpper
        | mir::RuntimeFunc::BytesLower
        | mir::RuntimeFunc::BytesConcat
        | mir::RuntimeFunc::BytesRepeat
        | mir::RuntimeFunc::BytesFromHex
        | mir::RuntimeFunc::BytesContains => {
            bytes::compile_bytes_call(builder, dest, func, args, ctx)?;
            Ok(())
        }

        // Object operations (Union type dispatch)
        mir::RuntimeFunc::IsTruthy
        | mir::RuntimeFunc::ObjContains
        | mir::RuntimeFunc::ObjToStr
        | mir::RuntimeFunc::ObjDefaultRepr
        | mir::RuntimeFunc::ObjAdd
        | mir::RuntimeFunc::ObjSub
        | mir::RuntimeFunc::ObjMul
        | mir::RuntimeFunc::ObjDiv
        | mir::RuntimeFunc::ObjFloorDiv
        | mir::RuntimeFunc::ObjMod
        | mir::RuntimeFunc::ObjPow
        | mir::RuntimeFunc::AnyGetItem => {
            object::compile_object_call(builder, dest, func, args, ctx)?;
            Ok(())
        }

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

        // File I/O operations
        mir::RuntimeFunc::FileOpen
        | mir::RuntimeFunc::FileRead
        | mir::RuntimeFunc::FileReadN
        | mir::RuntimeFunc::FileReadline
        | mir::RuntimeFunc::FileReadlines
        | mir::RuntimeFunc::FileWrite
        | mir::RuntimeFunc::FileClose
        | mir::RuntimeFunc::FileFlush
        | mir::RuntimeFunc::FileEnter
        | mir::RuntimeFunc::FileExit
        | mir::RuntimeFunc::FileIsClosed
        | mir::RuntimeFunc::FileName => {
            file::compile_file_call(builder, dest, func, args, ctx)?;
            Ok(())
        }

        // Exception-related operations
        mir::RuntimeFunc::ExcIsinstanceClass
        | mir::RuntimeFunc::ExcRaiseCustom
        | mir::RuntimeFunc::ExcRegisterClassName
        | mir::RuntimeFunc::ExcInstanceStr => {
            compile_exception_call(builder, dest, func, args, ctx)?;
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

use crate::utils::{create_raw_string_data, declare_runtime_function, load_operand};
use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::{AbiParam, InstBuilder};
use cranelift_codegen::isa::CallConv;
use cranelift_module::Module;

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
