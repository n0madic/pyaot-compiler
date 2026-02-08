//! Accessor methods for Lowering context internal state
//!
//! These methods provide controlled access to the Lowering context's internal state.
//! They encapsulate common access patterns and reduce tight coupling between modules.

use indexmap::IndexMap;
use pyaot_diagnostics::CompilerWarning;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::{BlockId, ClassId, FuncId, InternedString, LocalId, VarId};

use crate::narrowing::DeadBranch;

use super::{CrossModuleClassInfo, LoweredClassInfo, Lowering};

// =============================================================================
// String Interning
// =============================================================================

impl<'a> Lowering<'a> {
    /// Intern a string, returning an InternedString handle.
    pub(crate) fn intern(&mut self, s: &str) -> InternedString {
        self.interner.intern(s)
    }

    /// Resolve an InternedString to its string value.
    pub(crate) fn resolve(&self, s: InternedString) -> &str {
        self.interner.resolve(s)
    }

    /// Look up a string in the interner without interning it.
    pub(crate) fn lookup_interned(&self, s: &str) -> Option<InternedString> {
        self.interner.lookup(s)
    }
}

// =============================================================================
// Variable Mapping (var_to_local, var_types)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Get the LocalId for a variable, if it exists.
    pub(crate) fn get_var_local(&self, var_id: &VarId) -> Option<LocalId> {
        self.var_to_local.get(var_id).copied()
    }

    /// Map a variable to a local.
    pub(crate) fn insert_var_local(&mut self, var_id: VarId, local_id: LocalId) {
        self.var_to_local.insert(var_id, local_id);
    }

    /// Get the type for a variable, if tracked.
    /// Checks both local var_types and global_var_types.
    pub(crate) fn get_var_type(&self, var_id: &VarId) -> Option<&Type> {
        // First check local var_types
        self.var_types
            .get(var_id)
            // Fall back to global_var_types for global variables
            .or_else(|| self.global_var_types.get(var_id))
    }

    /// Set the type for a variable.
    /// For global variables, also stores the type in global_var_types for persistence.
    pub(crate) fn insert_var_type(&mut self, var_id: VarId, ty: Type) {
        self.var_types.insert(var_id, ty.clone());
        // If this is a global variable, also store in global_var_types
        if self.globals.contains(&var_id) {
            self.global_var_types.insert(var_id, ty);
        }
    }
}

// =============================================================================
// Basic Block Management (current_blocks, current_block_idx)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Push a new block and make it the current block.
    pub(crate) fn push_block(&mut self, block: mir::BasicBlock) {
        self.current_blocks.push(block);
        self.current_block_idx = self.current_blocks.len() - 1;
    }
}

// =============================================================================
// Loop Stack (loop_stack)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Push a loop context (continue_target, break_target) onto the stack.
    pub(crate) fn push_loop(&mut self, continue_target: BlockId, break_target: BlockId) {
        self.loop_stack.push((continue_target, break_target));
    }

    /// Pop the current loop context.
    pub(crate) fn pop_loop(&mut self) {
        self.loop_stack.pop();
    }

    /// Get the current loop context, if any.
    pub(crate) fn current_loop(&self) -> Option<(BlockId, BlockId)> {
        self.loop_stack.last().copied()
    }
}

// =============================================================================
// Function References (var_to_func)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Get the FuncId for a variable that holds a function reference.
    pub(crate) fn get_var_func(&self, var_id: &VarId) -> Option<FuncId> {
        self.var_to_func.get(var_id).copied()
    }

    /// Track that a variable holds a function reference.
    pub(crate) fn insert_var_func(&mut self, var_id: VarId, func_id: FuncId) {
        self.var_to_func.insert(var_id, func_id);
    }

    /// Check if a variable holds a function reference.
    pub(crate) fn has_var_func(&self, var_id: &VarId) -> bool {
        self.var_to_func.contains_key(var_id)
    }
}

// =============================================================================
// Closure Tracking (var_to_closure, var_to_wrapper, wrapper_func_ids)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Get the closure (FuncId, captures) for a variable.
    pub(crate) fn get_var_closure(&self, var_id: &VarId) -> Option<&(FuncId, Vec<hir::ExprId>)> {
        self.var_to_closure.get(var_id)
    }

    /// Track that a variable holds a closure.
    pub(crate) fn insert_var_closure(
        &mut self,
        var_id: VarId,
        func_id: FuncId,
        captures: Vec<hir::ExprId>,
    ) {
        self.var_to_closure.insert(var_id, (func_id, captures));
    }

    /// Check if a variable holds a closure.
    pub(crate) fn has_var_closure(&self, var_id: &VarId) -> bool {
        self.var_to_closure.contains_key(var_id)
    }

    /// Get the wrapper/original func pair for a variable that holds a decorator wrapper.
    pub(crate) fn get_var_wrapper(&self, var_id: &VarId) -> Option<(FuncId, FuncId)> {
        self.var_to_wrapper.get(var_id).copied()
    }

    /// Track that a variable holds a decorator wrapper closure.
    pub(crate) fn insert_var_wrapper(
        &mut self,
        var_id: VarId,
        wrapper_func_id: FuncId,
        original_func_id: FuncId,
    ) {
        self.var_to_wrapper
            .insert(var_id, (wrapper_func_id, original_func_id));
    }

    /// Register a function as a wrapper function (closure returned by decorator).
    pub(crate) fn insert_wrapper_func_id(&mut self, func_id: FuncId) {
        self.wrapper_func_ids.insert(func_id);
    }

    /// Track that a module-level variable holds a decorator wrapper closure.
    /// This persists across function lowering (unlike var_to_wrapper which is cleared per function).
    pub(crate) fn insert_module_var_wrapper(
        &mut self,
        var_id: VarId,
        wrapper_func_id: FuncId,
        original_func_id: FuncId,
    ) {
        self.module_var_wrappers
            .insert(var_id, (wrapper_func_id, original_func_id));
    }

    /// Get the wrapper/original func pair for a module-level variable that holds a decorator wrapper.
    pub(crate) fn get_module_var_wrapper(&self, var_id: &VarId) -> Option<(FuncId, FuncId)> {
        self.module_var_wrappers.get(var_id).copied()
    }

    /// Track that a module-level variable holds a function reference.
    /// This persists across function lowering (unlike var_to_func which is cleared per function).
    pub(crate) fn insert_module_var_func(&mut self, var_id: VarId, func_id: FuncId) {
        self.module_var_funcs.insert(var_id, func_id);
    }

    /// Get the function reference for a module-level variable.
    pub(crate) fn get_module_var_func(&self, var_id: &VarId) -> Option<FuncId> {
        self.module_var_funcs.get(var_id).copied()
    }
}

// =============================================================================
// Function Pointer Parameters (func_ptr_params)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Check if a variable is a function pointer parameter.
    pub(crate) fn is_func_ptr_param(&self, var_id: &VarId) -> bool {
        self.func_ptr_params.contains(var_id)
    }

    /// Insert a function pointer parameter.
    pub(crate) fn insert_func_ptr_param(&mut self, var_id: VarId) {
        self.func_ptr_params.insert(var_id);
    }

    /// Check if a function is a wrapper function.
    pub(crate) fn is_wrapper_func(&self, func_id: &FuncId) -> bool {
        self.wrapper_func_ids.contains(func_id)
    }
}

// =============================================================================
// Class Info (class_info, class_name_map)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Get class info by ClassId.
    pub(crate) fn get_class_info(&self, class_id: &ClassId) -> Option<&LoweredClassInfo> {
        self.class_info.get(class_id)
    }

    /// Insert class info for a ClassId.
    pub(crate) fn insert_class_info(&mut self, class_id: ClassId, info: LoweredClassInfo) {
        self.class_info.insert(class_id, info);
    }

    /// Check if a class exists.
    pub(crate) fn has_class(&self, class_id: &ClassId) -> bool {
        self.class_info.contains_key(class_id)
    }

    /// Get ClassId by class name.
    pub(crate) fn get_class_by_name(&self, name: &str) -> Option<ClassId> {
        self.class_name_map.get(name).copied()
    }

    /// Register a class name to ClassId mapping.
    pub(crate) fn register_class_name(&mut self, name: String, class_id: ClassId) {
        self.class_name_map.insert(name, class_id);
    }

    /// Iterate over all class info entries.
    pub(crate) fn class_info_iter(&self) -> impl Iterator<Item = (&ClassId, &LoweredClassInfo)> {
        self.class_info.iter()
    }
}

// =============================================================================
// Global Variables (globals)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Check if a variable is a global.
    pub(crate) fn is_global(&self, var_id: &VarId) -> bool {
        self.globals.contains(var_id)
    }
}

// =============================================================================
// Cell Variables (cell_vars, nonlocal_cells)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Check if a variable is a cell variable.
    pub(crate) fn is_cell_var(&self, var_id: &VarId) -> bool {
        self.cell_vars.contains(var_id)
    }

    /// Get the cell local for a nonlocal variable.
    pub(crate) fn get_nonlocal_cell(&self, var_id: &VarId) -> Option<LocalId> {
        self.nonlocal_cells.get(var_id).copied()
    }

    /// Map a nonlocal variable to its cell local.
    pub(crate) fn insert_nonlocal_cell(&mut self, var_id: VarId, local_id: LocalId) {
        self.nonlocal_cells.insert(var_id, local_id);
    }

    /// Check if a variable has a nonlocal cell mapping.
    pub(crate) fn has_nonlocal_cell(&self, var_id: &VarId) -> bool {
        self.nonlocal_cells.contains_key(var_id)
    }

    /// Clone the nonlocal cells mapping (for saving/restoring state).
    pub(crate) fn clone_nonlocal_cells(&self) -> IndexMap<VarId, LocalId> {
        self.nonlocal_cells.clone()
    }

    /// Restore nonlocal cells from a saved state.
    pub(crate) fn restore_nonlocal_cells(&mut self, cells: IndexMap<VarId, LocalId>) {
        self.nonlocal_cells = cells;
    }
}

// =============================================================================
// Union Narrowing (narrowed_union_vars)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Get the original Union type for a narrowed variable.
    pub(crate) fn get_narrowed_union_type(&self, var_id: &VarId) -> Option<Type> {
        self.narrowed_union_vars.get(var_id).cloned()
    }

    /// Track a narrowed Union variable with its original type.
    pub(crate) fn insert_narrowed_union(&mut self, var_id: VarId, original_type: Type) {
        self.narrowed_union_vars.insert(var_id, original_type);
    }

    /// Remove a narrowed Union variable tracking.
    pub(crate) fn remove_narrowed_union(&mut self, var_id: &VarId) {
        self.narrowed_union_vars.shift_remove(var_id);
    }
}

// =============================================================================
// Function Return Types (func_return_types, current_func_return_type)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Get the return type for a function.
    pub(crate) fn get_func_return_type(&self, func_id: &FuncId) -> Option<&Type> {
        self.func_return_types.get(func_id)
    }

    /// Set the return type for a function.
    pub(crate) fn insert_func_return_type(&mut self, func_id: FuncId, ty: Type) {
        self.func_return_types.insert(func_id, ty);
    }

    /// Get the current function's return type.
    pub(crate) fn get_current_func_return_type(&self) -> Option<&Type> {
        self.current_func_return_type.as_ref()
    }
}

// =============================================================================
// Closure Capture Types (closure_capture_types)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Get closure capture types for a function.
    pub(crate) fn get_closure_capture_types(&self, func_id: &FuncId) -> Option<&Vec<Type>> {
        self.closure_capture_types.get(func_id)
    }

    /// Set closure capture types for a function.
    pub(crate) fn insert_closure_capture_types(&mut self, func_id: FuncId, types: Vec<Type>) {
        self.closure_capture_types.insert(func_id, types);
    }

    /// Check if closure capture types are tracked for a function.
    pub(crate) fn has_closure_capture_types(&self, func_id: &FuncId) -> bool {
        self.closure_capture_types.contains_key(func_id)
    }
}

// =============================================================================
// Lambda Parameter Type Hints (lambda_param_type_hints)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Get caller-provided parameter type hints for a lambda.
    pub(crate) fn get_lambda_param_type_hints(&self, func_id: &FuncId) -> Option<&Vec<Type>> {
        self.lambda_param_type_hints.get(func_id)
    }

    /// Set parameter type hints for a lambda (e.g., reduce provides element type for both params).
    pub(crate) fn insert_lambda_param_type_hints(&mut self, func_id: FuncId, types: Vec<Type>) {
        self.lambda_param_type_hints.insert(func_id, types);
    }
}

// =============================================================================
// Module Exports (module_var_exports, module_func_exports, module_class_exports, cross_module_class_info)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Get a module variable export.
    pub(crate) fn get_module_var_export(&self, key: &(String, String)) -> Option<&(VarId, Type)> {
        self.module_var_exports.get(key)
    }

    /// Get a module function export (return type).
    pub(crate) fn get_module_func_export(&self, key: &(String, String)) -> Option<&Type> {
        self.module_func_exports.get(key)
    }

    /// Get a module class export (ClassId, class_name).
    pub(crate) fn get_module_class_export(
        &self,
        key: &(String, String),
    ) -> Option<&(ClassId, String)> {
        self.module_class_exports.get(key)
    }

    /// Iterate over all module class exports.
    pub(crate) fn module_class_exports_iter(
        &self,
    ) -> impl Iterator<Item = (&(String, String), &(ClassId, String))> {
        self.module_class_exports.iter()
    }

    /// Get cross-module class info.
    pub(crate) fn get_cross_module_class_info(
        &self,
        class_id: &ClassId,
    ) -> Option<&CrossModuleClassInfo> {
        self.cross_module_class_info.get(class_id)
    }
}

// =============================================================================
// Expression Type Cache (expr_type_cache)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Get a cached expression type.
    pub(crate) fn get_cached_expr_type(&self, expr_id: &hir::ExprId) -> Option<Type> {
        self.expr_type_cache.borrow().get(expr_id).cloned()
    }

    /// Cache an expression type.
    pub(crate) fn cache_expr_type(&self, expr_id: hir::ExprId, ty: Type) {
        self.expr_type_cache.borrow_mut().insert(expr_id, ty);
    }
}

// =============================================================================
// Default Value Slots (default_value_slots)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Get the global slot for a mutable default parameter.
    pub(crate) fn get_default_slot(&self, key: &(FuncId, usize)) -> Option<u32> {
        self.default_value_slots.get(key).copied()
    }
}

// =============================================================================
// Pending Varargs/Kwargs (pending_varargs_from_unpack, pending_kwargs_from_unpack)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Set the pending varargs tuple from list unpacking.
    pub(crate) fn set_pending_varargs(&mut self, local_id: LocalId) {
        self.pending_varargs_from_unpack = Some(local_id);
    }

    /// Take the pending varargs tuple.
    pub(crate) fn take_pending_varargs(&mut self) -> Option<LocalId> {
        self.pending_varargs_from_unpack.take()
    }

    /// Set the pending kwargs dict from **kwargs unpacking.
    pub(crate) fn set_pending_kwargs(&mut self, local_id: LocalId, value_type: Type) {
        self.pending_kwargs_from_unpack = Some((local_id, value_type));
    }

    /// Take the pending kwargs dict.
    pub(crate) fn take_pending_kwargs(&mut self) -> Option<(LocalId, Type)> {
        self.pending_kwargs_from_unpack.take()
    }

    /// Clear the pending kwargs without taking.
    pub(crate) fn clear_pending_kwargs(&mut self) {
        self.pending_kwargs_from_unpack = None;
    }
}

// =============================================================================
// MIR Module (mir_module)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Add a vtable to the MIR module.
    pub(crate) fn add_vtable(&mut self, vtable: mir::VtableInfo) {
        self.mir_module.vtables.push(vtable);
    }
}

// =============================================================================
// Warnings
// =============================================================================

impl<'a> Lowering<'a> {
    /// Emit a dead code warning for unreachable isinstance branches.
    ///
    /// This is called when type narrowing detects that an isinstance check
    /// produces Type::Never, indicating the branch can never be taken.
    pub(crate) fn emit_dead_code_warning(
        &mut self,
        span: pyaot_utils::Span,
        var_name: &str,
        checked_type: &Type,
        branch: DeadBranch,
    ) {
        let message = match branch {
            DeadBranch::ThenBranch => format!(
                "isinstance check is always False: variable '{}' cannot be type '{}'",
                var_name, checked_type
            ),
            DeadBranch::ElseBranch => format!(
                "isinstance check is always True: variable '{}' is already type '{}'",
                var_name, checked_type
            ),
        };

        self.warnings.add(CompilerWarning::dead_code(message, span));
    }

    /// Take collected warnings, leaving an empty collection.
    pub fn take_warnings(&mut self) -> pyaot_diagnostics::CompilerWarnings {
        std::mem::take(&mut self.warnings)
    }

    /// Check if there are any warnings.
    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }
}
