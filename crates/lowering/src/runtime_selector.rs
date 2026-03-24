//! Runtime function selection based on type information
//!
//! This module provides methods for selecting the appropriate runtime functions
//! based on variable types. It handles global variable storage, cell storage for
//! closures, and class attribute storage.

use pyaot_mir::{self as mir, ValueKind};
use pyaot_types::Type;

use crate::Lowering;

impl<'a> Lowering<'a> {
    // ==================== Type to ValueKind Conversion ====================

    /// Convert Type to ValueKind for storage operations
    fn type_to_value_kind(&self, var_type: &Type) -> ValueKind {
        match var_type {
            Type::Int => ValueKind::Int,
            Type::Float => ValueKind::Float,
            // None uses Bool (i8) storage: None is always represented as 0,
            // the same bit pattern as `false`, so both fit in i8 storage.
            Type::Bool | Type::None => ValueKind::Bool,
            // Heap types (str, list, dict, tuple, etc.) use pointer storage
            Type::Str
            | Type::List(_)
            | Type::Dict(_, _)
            | Type::Tuple(_)
            | Type::Set(_)
            | Type::Bytes
            | Type::Class { .. }
            | Type::Iterator(_)
            | Type::Union(_)
            | Type::RuntimeObject(_)
            | Type::File
            | Type::Any
            | Type::BuiltinException(_) => ValueKind::Ptr,
            // Compile-time-only types that should not appear in storage operations.
            // Explicit matches ensure new Type variants trigger exhaustiveness errors.
            Type::Function { .. } | Type::Var(_) | Type::Never => ValueKind::Int,
        }
    }

    // ==================== Global Variable Storage ====================

    /// Get the appropriate runtime function for setting a global variable based on its type
    pub(crate) fn get_global_set_func(&self, var_type: &Type) -> mir::RuntimeFunc {
        mir::RuntimeFunc::GlobalSet(self.type_to_value_kind(var_type))
    }

    /// Get the appropriate runtime function for getting a global variable based on its type
    pub(crate) fn get_global_get_func(&self, var_type: &Type) -> mir::RuntimeFunc {
        mir::RuntimeFunc::GlobalGet(self.type_to_value_kind(var_type))
    }

    // ==================== Cell Storage (for closures/nonlocal) ====================

    /// Get the appropriate runtime function for creating a cell based on type
    pub(crate) fn get_make_cell_func(&self, var_type: &Type) -> mir::RuntimeFunc {
        mir::RuntimeFunc::MakeCell(self.type_to_value_kind(var_type))
    }

    /// Get the appropriate runtime function for getting a value from a cell based on type
    pub(crate) fn get_cell_get_func(&self, var_type: &Type) -> mir::RuntimeFunc {
        mir::RuntimeFunc::CellGet(self.type_to_value_kind(var_type))
    }

    /// Get the appropriate runtime function for setting a value in a cell based on type
    pub(crate) fn get_cell_set_func(&self, var_type: &Type) -> mir::RuntimeFunc {
        mir::RuntimeFunc::CellSet(self.type_to_value_kind(var_type))
    }

    // ==================== Class Attribute Storage ====================

    /// Get the appropriate runtime function for setting a class attribute based on type
    pub(crate) fn get_class_attr_set_func(&self, attr_type: &Type) -> mir::RuntimeFunc {
        mir::RuntimeFunc::ClassAttrSet(self.type_to_value_kind(attr_type))
    }

    /// Get the appropriate runtime function for getting a class attribute based on type
    pub(crate) fn get_class_attr_get_func(&self, attr_type: &Type) -> mir::RuntimeFunc {
        mir::RuntimeFunc::ClassAttrGet(self.type_to_value_kind(attr_type))
    }
}
