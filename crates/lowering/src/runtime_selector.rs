//! Runtime function selection based on type information
//!
//! This module provides methods for selecting the appropriate runtime functions
//! based on variable types. It handles global variable storage, cell storage for
//! closures, and class attribute storage.
//!
//! Phase 2 §F.6: the per-storage-kind enum is gone; each selector inlines
//! the four-arm dispatch directly via `pick_storage_def`. Once the runtime
//! collapses to uniform `Value`-typed externs (a follow-up cleanup), these
//! helpers reduce to a single constant return.

use pyaot_core_defs::runtime_func_def::*;
use pyaot_core_defs::RuntimeFuncDef;
use pyaot_mir as mir;
use pyaot_types::Type;

use crate::Lowering;

/// Returns true if the type stores its value as a raw heap pointer
/// (`*mut Obj`). Compile-time-only types route through the `Int` arm so
/// they never reach storage operations in practice.
fn is_ptr_storage(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Str
            | Type::List(_)
            | Type::Dict(_, _)
            | Type::DefaultDict(_, _)
            | Type::Tuple(_)
            | Type::TupleVar(_)
            | Type::Set(_)
            | Type::Bytes
            | Type::Class { .. }
            | Type::Iterator(_)
            | Type::Union(_)
            | Type::RuntimeObject(_)
            | Type::File(_)
            | Type::Any
            | Type::HeapAny
            | Type::BuiltinException(_)
            | Type::NotImplementedT
    )
}

/// Pick one of four typed RuntimeFuncDef references based on storage kind.
/// `int_def` covers Int and the unreachable Function/Var/Never sentinels.
fn pick_storage_def(
    ty: &Type,
    int_def: &'static RuntimeFuncDef,
    float_def: &'static RuntimeFuncDef,
    bool_def: &'static RuntimeFuncDef,
    ptr_def: &'static RuntimeFuncDef,
) -> &'static RuntimeFuncDef {
    match ty {
        Type::Int => int_def,
        Type::Float => float_def,
        // None uses Bool (i8) storage: None is always represented as 0,
        // the same bit pattern as `false`, so both fit in i8 storage.
        Type::Bool | Type::None => bool_def,
        _ if is_ptr_storage(ty) => ptr_def,
        // Compile-time-only types that should not appear in storage operations.
        Type::Function { .. } | Type::Var(_) | Type::Never => int_def,
        // Exhaustiveness fallback — every Type variant is handled above; if a
        // new variant lands without a storage arm, route through Int rather
        // than panicking and document via clippy/test breakage.
        _ => int_def,
    }
}

impl<'a> Lowering<'a> {
    // ==================== Global Variable Storage ====================

    /// Get the appropriate runtime function for setting a global variable based on its type
    pub(crate) fn get_global_set_func(&self, var_type: &Type) -> mir::RuntimeFunc {
        mir::RuntimeFunc::Call(pick_storage_def(
            var_type,
            &RT_GLOBAL_SET_INT,
            &RT_GLOBAL_SET_FLOAT,
            &RT_GLOBAL_SET_BOOL,
            &RT_GLOBAL_SET_PTR,
        ))
    }

    /// Get the appropriate runtime function for getting a global variable based on its type
    pub(crate) fn get_global_get_func(&self, var_type: &Type) -> mir::RuntimeFunc {
        mir::RuntimeFunc::Call(pick_storage_def(
            var_type,
            &RT_GLOBAL_GET_INT,
            &RT_GLOBAL_GET_FLOAT,
            &RT_GLOBAL_GET_BOOL,
            &RT_GLOBAL_GET_PTR,
        ))
    }

    // ==================== Cell Storage (for closures/nonlocal) ====================

    /// Get the appropriate runtime function for creating a cell based on type
    pub(crate) fn get_make_cell_func(&self, var_type: &Type) -> mir::RuntimeFunc {
        mir::RuntimeFunc::Call(pick_storage_def(
            var_type,
            &RT_MAKE_CELL_INT,
            &RT_MAKE_CELL_FLOAT,
            &RT_MAKE_CELL_BOOL,
            &RT_MAKE_CELL_PTR,
        ))
    }

    /// Get the appropriate runtime function for getting a value from a cell based on type
    pub(crate) fn get_cell_get_func(&self, var_type: &Type) -> mir::RuntimeFunc {
        mir::RuntimeFunc::Call(pick_storage_def(
            var_type,
            &RT_CELL_GET_INT,
            &RT_CELL_GET_FLOAT,
            &RT_CELL_GET_BOOL,
            &RT_CELL_GET_PTR,
        ))
    }

    /// Get the appropriate runtime function for setting a value in a cell based on type
    pub(crate) fn get_cell_set_func(&self, var_type: &Type) -> mir::RuntimeFunc {
        mir::RuntimeFunc::Call(pick_storage_def(
            var_type,
            &RT_CELL_SET_INT,
            &RT_CELL_SET_FLOAT,
            &RT_CELL_SET_BOOL,
            &RT_CELL_SET_PTR,
        ))
    }

    // ==================== Class Attribute Storage ====================

    /// Get the appropriate runtime function for setting a class attribute based on type
    pub(crate) fn get_class_attr_set_func(&self, attr_type: &Type) -> mir::RuntimeFunc {
        mir::RuntimeFunc::Call(pick_storage_def(
            attr_type,
            &RT_CLASS_ATTR_SET_INT,
            &RT_CLASS_ATTR_SET_FLOAT,
            &RT_CLASS_ATTR_SET_BOOL,
            &RT_CLASS_ATTR_SET_PTR,
        ))
    }

    /// Get the appropriate runtime function for getting a class attribute based on type
    pub(crate) fn get_class_attr_get_func(&self, attr_type: &Type) -> mir::RuntimeFunc {
        mir::RuntimeFunc::Call(pick_storage_def(
            attr_type,
            &RT_CLASS_ATTR_GET_INT,
            &RT_CLASS_ATTR_GET_FLOAT,
            &RT_CLASS_ATTR_GET_BOOL,
            &RT_CLASS_ATTR_GET_PTR,
        ))
    }
}
