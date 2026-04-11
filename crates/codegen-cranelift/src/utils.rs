//! Utility functions for Cranelift code generation

use cranelift_codegen::ir::types as cltypes;
use cranelift_codegen::ir::{Inst, InstBuilder, MemFlags, Signature, Value};
use cranelift_frontend::{FunctionBuilder, Variable};
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module};
use cranelift_object::ObjectModule;
use indexmap::IndexMap;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_mir::{self as mir, Operand};
use pyaot_types::Type;
use pyaot_utils::{InternedString, LocalId, StringInterner};

/// Helper to declare a runtime function with proper error handling
pub fn declare_runtime_function(
    module: &mut ObjectModule,
    name: &str,
    sig: &Signature,
) -> Result<FuncId> {
    module
        .declare_function(name, Linkage::Import, sig)
        .map_err(|e| {
            CompilerError::codegen_error(
                format!("Failed to declare runtime function '{}': {}", name, e),
                None,
            )
        })
}

/// Get first result from call instruction with clear error message
pub fn get_call_result(builder: &FunctionBuilder, call_inst: Inst) -> Value {
    *builder
        .inst_results(call_inst)
        .first()
        .expect("internal compiler error: call instruction should have return value")
}

/// Convert MIR type to Cranelift type
pub fn type_to_cranelift(ty: &Type) -> cltypes::Type {
    match ty {
        Type::Int => cltypes::I64,
        Type::Float => cltypes::F64,
        Type::Bool => cltypes::I8,
        Type::None => cltypes::I8,
        Type::Str | Type::List(_) | Type::Dict(_, _) | Type::Tuple(_) | Type::File => {
            cltypes::I64 // Pointer to heap object
        }
        // Union values are stored as boxed pointers (*mut Obj)
        Type::Union(_) => cltypes::I64,
        Type::Never => panic!("type_to_cranelift: Never type should not reach codegen"),
        // All remaining types are pointer-sized: Set, Bytes, Optional, Any, HeapAny,
        // Class, BuiltinException, RuntimeObject, Iterator, Function, Var, etc.
        _ => cltypes::I64,
    }
}

/// Mangle function names to avoid conflicts with C reserved names.
/// Reserved names like "main" are prefixed with "__pyuser_" to avoid
/// conflicts with the C entry point we generate.
pub fn mangle_function_name(name: &str) -> String {
    // Reserved names that conflict with C runtime
    const RESERVED_NAMES: &[&str] = &["main", "_main", "_start"];

    if RESERVED_NAMES.contains(&name) {
        format!("__pyuser_{}", name)
    } else {
        name.to_string()
    }
}

/// Load an operand into a Cranelift value
pub fn load_operand(
    builder: &mut FunctionBuilder,
    operand: &Operand,
    var_map: &IndexMap<LocalId, Variable>,
) -> cranelift_codegen::ir::Value {
    match operand {
        Operand::Local(local_id) => {
            let var = *var_map
                .get(local_id)
                .expect("internal error: local not in var_map - codegen bug");
            builder.use_var(var)
        }
        Operand::Constant(c) => match c {
            mir::Constant::Int(i) => builder.ins().iconst(cltypes::I64, *i),
            mir::Constant::Float(f) => builder.ins().f64const(*f),
            mir::Constant::Bool(b) => builder.ins().iconst(cltypes::I8, *b as i64),
            mir::Constant::None => builder.ins().iconst(cltypes::I8, 0),
            _ => builder.ins().iconst(cltypes::I64, 0),
        },
    }
}

/// Load an operand and coerce it to a target Cranelift type
/// Handles i8 -> i64 extension, i64 -> i8 reduction, and f64 <-> i64 bitcast
pub fn load_operand_as(
    builder: &mut FunctionBuilder,
    operand: &Operand,
    var_map: &IndexMap<LocalId, Variable>,
    target_type: cltypes::Type,
) -> cranelift_codegen::ir::Value {
    let val = load_operand(builder, operand, var_map);
    let val_type = builder.func.dfg.value_type(val);

    if val_type == target_type {
        return val;
    }

    match (val_type, target_type) {
        // i8 to i64 - unsigned extend (for bool values stored as raw)
        (cltypes::I8, cltypes::I64) => builder.ins().uextend(cltypes::I64, val),
        // i64 to i8 - reduce
        (cltypes::I64, cltypes::I8) => builder.ins().ireduce(cltypes::I8, val),
        // i64 to i32 - reduce (for var_id, generator index/state)
        (cltypes::I64, cltypes::I32) => builder.ins().ireduce(cltypes::I32, val),
        // i32 to i64 - unsigned extend
        (cltypes::I32, cltypes::I64) => builder.ins().uextend(cltypes::I64, val),
        // f64 to i64 - bitcast (for storing floats in generic list/dict slots)
        (cltypes::F64, cltypes::I64) => builder.ins().bitcast(cltypes::I64, MemFlags::new(), val),
        // i64 to f64 - bitcast (for loading floats from generic list/dict slots)
        (cltypes::I64, cltypes::F64) => builder.ins().bitcast(cltypes::F64, MemFlags::new(), val),
        // Other cases - return as-is (caller's responsibility to handle)
        _ => val,
    }
}

/// Internal helper to create a data section with given bytes and prefix
fn create_data_section_impl(
    module: &mut ObjectModule,
    bytes: Vec<u8>,
    counter: &std::sync::atomic::AtomicUsize,
    prefix: &str,
) -> DataId {
    use std::sync::atomic::Ordering;

    // Relaxed is sufficient: compilation is single-threaded, so no cross-thread
    // visibility ordering is required — only atomicity for the static counter.
    let id = counter.fetch_add(1, Ordering::Relaxed);
    let data_name = format!("{prefix}{id}");

    let data_id = module
        .declare_data(&data_name, Linkage::Local, false, false)
        .expect("Failed to declare data section - this should never fail for local data");

    let mut desc = DataDescription::new();
    desc.define(bytes.into_boxed_slice());
    module
        .define_data(data_id, &desc)
        .expect("Failed to define data section - this should never fail");

    data_id
}

/// Create a data section containing a null-terminated string
pub fn create_string_data(
    module: &mut ObjectModule,
    s: InternedString,
    interner: &StringInterner,
) -> DataId {
    use std::sync::atomic::AtomicUsize;
    static STRING_COUNTER: AtomicUsize = AtomicUsize::new(0);

    let str_content = interner.resolve(s);
    let mut bytes = str_content.as_bytes().to_vec();
    bytes.push(0); // null terminator

    create_data_section_impl(module, bytes, &STRING_COUNTER, "__str_")
}

/// Create a data section containing raw string bytes (no null terminator)
/// Used for rt_make_str which takes a length parameter
pub fn create_raw_string_data(
    module: &mut ObjectModule,
    s: InternedString,
    interner: &StringInterner,
) -> DataId {
    use std::sync::atomic::AtomicUsize;
    static RAW_STRING_COUNTER: AtomicUsize = AtomicUsize::new(0);

    let str_content = interner.resolve(s);
    let bytes = str_content.as_bytes().to_vec();

    create_data_section_impl(module, bytes, &RAW_STRING_COUNTER, "__rawstr_")
}

/// Create a data section containing raw string bytes for traceback info.
/// Used for function names and file names embedded in the binary.
pub fn create_traceback_string_data(module: &mut ObjectModule, s: &str) -> DataId {
    use std::sync::atomic::AtomicUsize;
    static TB_COUNTER: AtomicUsize = AtomicUsize::new(0);

    create_data_section_impl(module, s.as_bytes().to_vec(), &TB_COUNTER, "__tbstr_")
}

/// Create a data section containing raw bytes (no null terminator)
/// Used for rt_make_bytes which takes a length parameter
pub fn create_raw_bytes_data(module: &mut ObjectModule, bytes: &[u8]) -> DataId {
    use std::sync::atomic::AtomicUsize;
    static RAW_BYTES_COUNTER: AtomicUsize = AtomicUsize::new(0);

    create_data_section_impl(module, bytes.to_vec(), &RAW_BYTES_COUNTER, "__rawbytes_")
}

/// Helper to determine if an operand is a float type
pub fn is_float_operand(operand: &Operand, locals: &IndexMap<LocalId, mir::Local>) -> bool {
    match operand {
        Operand::Local(local_id) => locals
            .get(local_id)
            .is_some_and(|l| matches!(l.ty, Type::Float)),
        Operand::Constant(mir::Constant::Float(_)) => true,
        _ => false,
    }
}

/// Helper to determine if an operand is an int type
pub fn is_int_operand(operand: &Operand, locals: &IndexMap<LocalId, mir::Local>) -> bool {
    match operand {
        Operand::Local(local_id) => locals
            .get(local_id)
            .is_some_and(|l| matches!(l.ty, Type::Int)),
        Operand::Constant(mir::Constant::Int(_)) => true,
        _ => false,
    }
}

/// Helper to determine if an operand is a bool type
pub fn is_bool_operand(operand: &Operand, locals: &IndexMap<LocalId, mir::Local>) -> bool {
    match operand {
        Operand::Local(local_id) => locals
            .get(local_id)
            .is_some_and(|l| matches!(l.ty, Type::Bool)),
        Operand::Constant(mir::Constant::Bool(_)) => true,
        _ => false,
    }
}

/// Convert an int or bool value to float (for mixed-type arithmetic)
pub fn promote_to_float(
    builder: &mut FunctionBuilder,
    val: cranelift_codegen::ir::Value,
    operand: &Operand,
    locals: &IndexMap<LocalId, mir::Local>,
) -> cranelift_codegen::ir::Value {
    if is_int_operand(operand, locals) {
        // Convert signed int64 to float64
        builder.ins().fcvt_from_sint(cltypes::F64, val)
    } else if is_bool_operand(operand, locals) {
        // Bool is i8: extend to i64, then convert to f64
        let i64_val = builder.ins().uextend(cltypes::I64, val);
        builder.ins().fcvt_from_sint(cltypes::F64, i64_val)
    } else {
        val
    }
}
