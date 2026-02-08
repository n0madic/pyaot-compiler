//! Cell operations code generation (for nonlocal variables)

use cranelift_codegen::ir::types as cltypes;
use cranelift_frontend::FunctionBuilder;
use pyaot_diagnostics::Result;
use pyaot_mir::{self as mir, Operand, ValueKind};
use pyaot_utils::LocalId;

use crate::context::CodegenContext;
use crate::runtime_helpers::{compile_unary_runtime_call, compile_void_runtime_call};

/// Get the Cranelift type for a ValueKind
fn value_kind_to_cltype(kind: ValueKind) -> cltypes::Type {
    match kind {
        ValueKind::Int => cltypes::I64,
        ValueKind::Float => cltypes::F64,
        ValueKind::Bool => cltypes::I8,
        ValueKind::Ptr => cltypes::I64,
    }
}

/// Get the runtime function name for MakeCell
fn make_cell_func_name(kind: ValueKind) -> &'static str {
    match kind {
        ValueKind::Int => "rt_make_cell_int",
        ValueKind::Float => "rt_make_cell_float",
        ValueKind::Bool => "rt_make_cell_bool",
        ValueKind::Ptr => "rt_make_cell_ptr",
    }
}

/// Get the runtime function name for CellGet
fn cell_get_func_name(kind: ValueKind) -> &'static str {
    match kind {
        ValueKind::Int => "rt_cell_get_int",
        ValueKind::Float => "rt_cell_get_float",
        ValueKind::Bool => "rt_cell_get_bool",
        ValueKind::Ptr => "rt_cell_get_ptr",
    }
}

/// Get the runtime function name for CellSet
fn cell_set_func_name(kind: ValueKind) -> &'static str {
    match kind {
        ValueKind::Int => "rt_cell_set_int",
        ValueKind::Float => "rt_cell_set_float",
        ValueKind::Bool => "rt_cell_set_bool",
        ValueKind::Ptr => "rt_cell_set_ptr",
    }
}

/// Compile a cell-related runtime call (for nonlocal variable support)
pub fn compile_cell_call(
    builder: &mut FunctionBuilder,
    dest: LocalId,
    func: &mir::RuntimeFunc,
    args: &[Operand],
    ctx: &mut CodegenContext,
) -> Result<()> {
    match func {
        // MakeCell operations: value -> cell pointer (all need GC update)
        mir::RuntimeFunc::MakeCell(kind) => {
            compile_unary_runtime_call(
                builder,
                make_cell_func_name(*kind),
                value_kind_to_cltype(*kind),
                cltypes::I64,
                &args[0],
                dest,
                ctx,
                true, // Cell allocations need GC update
            )?;
        }

        // CellGet operations: cell pointer -> value
        mir::RuntimeFunc::CellGet(kind) => {
            let needs_gc = matches!(kind, ValueKind::Ptr);
            compile_unary_runtime_call(
                builder,
                cell_get_func_name(*kind),
                cltypes::I64,
                value_kind_to_cltype(*kind),
                &args[0],
                dest,
                ctx,
                needs_gc, // Only pointer results need GC update
            )?;
        }

        // CellSet operations: void (cell pointer, value)
        mir::RuntimeFunc::CellSet(kind) => {
            compile_void_runtime_call(
                builder,
                cell_set_func_name(*kind),
                &[cltypes::I64, value_kind_to_cltype(*kind)],
                args,
                ctx,
            )?;
        }

        _ => unreachable!("Non-cell function passed to compile_cell_call"),
    }

    Ok(())
}
