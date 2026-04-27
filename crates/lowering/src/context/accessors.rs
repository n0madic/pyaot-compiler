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

use super::{CrossModuleClassInfo, LoweredClassInfo, Lowering};
use crate::narrowing::DeadBranch;

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
// Variable Mapping (symbols.var_to_local, symbols.var_types)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Get a block-local shadow local emitted for a materialized narrowing.
    pub(crate) fn get_block_narrowed_local(&self, var_id: &VarId) -> Option<LocalId> {
        self.codegen
            .block_narrowed_locals
            .get(var_id)
            .map(|info| info.local_id)
    }

    /// Record the block-local shadow local for a materialized narrowing.
    pub(crate) fn insert_block_narrowed_local(
        &mut self,
        var_id: VarId,
        local_id: LocalId,
        storage_ty: Type,
        narrowed_ty: Type,
    ) {
        self.codegen.block_narrowed_locals.insert(
            var_id,
            super::BlockNarrowedLocal {
                local_id,
                storage_ty,
                narrowed_ty,
            },
        );
    }

    /// Drop a materialized narrowing local, typically after the variable is reassigned.
    pub(crate) fn remove_block_narrowed_local(&mut self, var_id: &VarId) {
        self.codegen.block_narrowed_locals.shift_remove(var_id);
    }

    /// If `var_id` currently has a block-local narrowed shadow, return the
    /// original pre-narrowing storage type that writes must target.
    pub(crate) fn get_block_narrowed_storage_type(&self, var_id: &VarId) -> Option<&Type> {
        self.codegen
            .block_narrowed_locals
            .get(var_id)
            .map(|info| &info.storage_ty)
    }

    /// Clear all per-block materialized narrowing locals.
    pub(crate) fn clear_block_narrowed_locals(&mut self) {
        self.codegen.block_narrowed_locals.clear();
    }

    /// Get the LocalId for a variable, if it exists.
    pub(crate) fn get_var_local(&self, var_id: &VarId) -> Option<LocalId> {
        self.symbols.var_to_local.get(var_id).copied()
    }

    /// Map a variable to a local.
    pub(crate) fn insert_var_local(&mut self, var_id: VarId, local_id: LocalId) {
        self.symbols.var_to_local.insert(var_id, local_id);
    }

    /// Get the type for a variable, if tracked.
    /// Checks local var_types, refined types, then global_var_types.
    pub(crate) fn get_var_type(&self, var_id: &VarId) -> Option<&Type> {
        self.symbols
            .var_types
            .get(var_id)
            .or_else(|| self.lowering_seed_info.refined_container_types.get(var_id))
            .or_else(|| self.symbols.global_var_types.get(var_id))
    }

    /// Read a variable's **base** type — fully independent of
    /// `symbols.var_types` (which is cleared per function and only
    /// tracks lowering-time writes). §1.4u-b step 4
    /// restricts this accessor to stable sources so `compute_expr_type`
    /// can be a pure function of HIR + F/M state, cacheable at
    /// module level.
    ///
    /// Fallback chain (all stable after `build_lowering_seed_info`
    /// completes, never touched by narrowing):
    /// 1. `base_var_types` — persistent per-module map seeded from
    ///    every function's annotated params, prescan locals, and
    ///    exception-handler binding types.
    /// 2. `refined_container_types` — empty-container refine output.
    /// 3. `current_local_seed_types` — current function's Area E §E.6 prescan.
    /// 4. `global_var_types` — module-level globals.
    ///
    /// Consumers that need the **effective** (narrowing-aware) type
    /// at a use site must go through `seed_expr_type` — its Var
    /// branch reads `get_var_type` first.
    pub(crate) fn get_base_var_type(&self, var_id: &VarId) -> Option<&Type> {
        self.lowering_seed_info
            .base_var_types
            .get(var_id)
            .or_else(|| self.lowering_seed_info.refined_container_types.get(var_id))
            .or_else(|| self.lowering_seed_info.current_local_seed_types.get(var_id))
            .or_else(|| self.symbols.global_var_types.get(var_id))
    }

    /// Set the type for a variable.
    /// For global variables, also stores the type in global_var_types for persistence.
    pub(crate) fn insert_var_type(&mut self, var_id: VarId, ty: Type) {
        self.symbols.var_types.insert(var_id, ty.clone());
        if self.symbols.globals.contains(&var_id) {
            self.symbols.global_var_types.insert(var_id, ty);
        }
    }

    /// Lightweight lowering-time seed type.
    ///
    /// Regular lowering mostly reads precomputed seed metadata for non-`Var`
    /// expressions and the current lowered view for `Var`s. A small set of
    /// context-sensitive expression kinds (`Attribute`, `BuiltinCall`) are
    /// recomputed against the current lowering-time var map so loop-carried
    /// locals like `v` in `zip(v._children, v._local_grads)` can refine after
    /// earlier `IterAdvance` binds in the same CFG block.
    ///
    /// Seed-building inside `type_planning` continues to use
    /// `seed_expr_type_by_id`.
    pub(crate) fn seed_expr_type(&self, expr_id: hir::ExprId, hir_module: &hir::Module) -> Type {
        let expr = &hir_module.exprs[expr_id];
        match &expr.kind {
            hir::ExprKind::Var(var_id) => self
                .codegen
                .block_narrowed_locals
                .get(var_id)
                .map(|info| info.narrowed_ty.clone())
                .or_else(|| self.get_var_type(var_id).cloned())
                .or_else(|| self.get_base_var_type(var_id).cloned())
                .or_else(|| expr.ty.clone())
                .unwrap_or(Type::Any),
            hir::ExprKind::Int(_) => Type::Int,
            hir::ExprKind::Float(_) => Type::Float,
            hir::ExprKind::Bool(_) => Type::Bool,
            hir::ExprKind::Str(_) => Type::Str,
            hir::ExprKind::Bytes(_) => Type::Bytes,
            hir::ExprKind::None => Type::None,
            hir::ExprKind::TypeRef(ty) => ty.clone(),
            hir::ExprKind::Attribute { obj, attr } => {
                let obj_ty = self.seed_expr_type(*obj, hir_module);
                self.attribute_result_type(&obj_ty, *attr, expr)
            }
            hir::ExprKind::BuiltinCall { builtin, args, .. } => {
                let arg_types: Vec<Type> = args
                    .iter()
                    .map(|arg_id| self.seed_expr_type(*arg_id, hir_module))
                    .collect();
                self.builtin_call_result_type(builtin, args, &arg_types, hir_module, expr)
            }
            _ => self
                .lowering_seed_info
                .lookup(expr_id)
                .cloned()
                .or_else(|| expr.ty.clone())
                .unwrap_or(Type::Any),
        }
    }

    pub(crate) fn get_refined_class_field_type(
        &self,
        class_id: &pyaot_utils::ClassId,
        field: &InternedString,
    ) -> Option<&Type> {
        self.lowering_seed_info
            .refined_class_field_types
            .get(class_id)
            .and_then(|fields| fields.get(field))
    }
}

// =============================================================================
// Basic Block Management (codegen.current_blocks, codegen.current_block_idx)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Push a new block and make it the current block.
    pub(crate) fn push_block(&mut self, block: mir::BasicBlock) {
        self.codegen.current_blocks.push(block);
        self.codegen.current_block_idx = self.codegen.current_blocks.len() - 1;
    }
}

// =============================================================================
// Loop Stack (codegen.loop_stack)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Push a loop context (continue_target, break_target) onto the stack.
    #[allow(dead_code)]
    pub(crate) fn push_loop(&mut self, continue_target: BlockId, break_target: BlockId) {
        self.codegen
            .loop_stack
            .push((continue_target, break_target));
    }

    /// Pop the current loop context.
    #[allow(dead_code)]
    pub(crate) fn pop_loop(&mut self) {
        self.codegen.loop_stack.pop();
    }

    /// Get the current loop context, if any.
    pub(crate) fn current_loop(&self) -> Option<(BlockId, BlockId)> {
        self.codegen.loop_stack.last().copied()
    }
}

// =============================================================================
// Function References (symbols.var_to_func)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Get the FuncId for a variable that holds a function reference.
    pub(crate) fn get_var_func(&self, var_id: &VarId) -> Option<FuncId> {
        self.symbols.var_to_func.get(var_id).copied()
    }

    /// Track that a variable holds a function reference.
    pub(crate) fn insert_var_func(&mut self, var_id: VarId, func_id: FuncId) {
        self.symbols.var_to_func.insert(var_id, func_id);
    }

    /// Check if a variable holds a function reference.
    pub(crate) fn has_var_func(&self, var_id: &VarId) -> bool {
        self.symbols.var_to_func.contains_key(var_id)
    }
}

// =============================================================================
// Closure Tracking (closures.*)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Get the closure (FuncId, captures) for a variable.
    pub(crate) fn get_var_closure(&self, var_id: &VarId) -> Option<&(FuncId, Vec<hir::ExprId>)> {
        self.closures.var_to_closure.get(var_id)
    }

    /// Track that a variable holds a closure.
    pub(crate) fn insert_var_closure(
        &mut self,
        var_id: VarId,
        func_id: FuncId,
        captures: Vec<hir::ExprId>,
    ) {
        self.closures
            .var_to_closure
            .insert(var_id, (func_id, captures));
    }

    /// Check if a variable holds a closure.
    pub(crate) fn has_var_closure(&self, var_id: &VarId) -> bool {
        self.closures.var_to_closure.contains_key(var_id)
    }

    /// Get the wrapper/original func pair for a variable that holds a decorator wrapper.
    pub(crate) fn get_var_wrapper(&self, var_id: &VarId) -> Option<(FuncId, FuncId)> {
        self.closures.var_to_wrapper.get(var_id).copied()
    }

    /// Track that a variable holds a decorator wrapper closure.
    pub(crate) fn insert_var_wrapper(
        &mut self,
        var_id: VarId,
        wrapper_func_id: FuncId,
        original_func_id: FuncId,
    ) {
        self.closures
            .var_to_wrapper
            .insert(var_id, (wrapper_func_id, original_func_id));
    }

    /// Register a function as a wrapper function (closure returned by decorator).
    pub(crate) fn insert_wrapper_func_id(&mut self, func_id: FuncId) {
        self.closures.wrapper_func_ids.insert(func_id);
    }

    /// Track that a module-level variable holds a decorator wrapper closure.
    pub(crate) fn insert_module_var_wrapper(
        &mut self,
        var_id: VarId,
        wrapper_func_id: FuncId,
        original_func_id: FuncId,
    ) {
        self.modules
            .module_var_wrappers
            .insert(var_id, (wrapper_func_id, original_func_id));
    }

    /// Get the wrapper/original func pair for a module-level variable.
    pub(crate) fn get_module_var_wrapper(&self, var_id: &VarId) -> Option<(FuncId, FuncId)> {
        self.modules.module_var_wrappers.get(var_id).copied()
    }

    /// Track that a module-level variable holds a function reference.
    pub(crate) fn insert_module_var_func(&mut self, var_id: VarId, func_id: FuncId) {
        self.modules.module_var_funcs.insert(var_id, func_id);
    }

    /// Get the function reference for a module-level variable.
    pub(crate) fn get_module_var_func(&self, var_id: &VarId) -> Option<FuncId> {
        self.modules.module_var_funcs.get(var_id).copied()
    }
}

// =============================================================================
// Function Pointer Parameters (closures.func_ptr_params)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Check if a variable is a function pointer parameter.
    pub(crate) fn is_func_ptr_param(&self, var_id: &VarId) -> bool {
        self.closures.func_ptr_params.contains(var_id)
    }

    /// Insert a function pointer parameter.
    pub(crate) fn insert_func_ptr_param(&mut self, var_id: VarId) {
        self.closures.func_ptr_params.insert(var_id);
    }

    /// Check if a function is a wrapper function.
    pub(crate) fn is_wrapper_func(&self, func_id: &FuncId) -> bool {
        self.closures.wrapper_func_ids.contains(func_id)
    }

    /// §P.2.2: if `func_id` is a wrapper (closure returned by a decorator),
    /// return the index of its fn-ptr parameter — which is also the matching
    /// capture-tuple slot index, since closure captures become the leading
    /// params of the callee. The producer-side closure-construction site
    /// uses this to decide whether to `ValueFromInt`-wrap a capture; the
    /// callee's prologue uses the same predicate to emit `UnwrapValueInt`.
    /// Driving both sides off `wrapper_func_ids` + `wrapper_func_param_name`
    /// keeps producer/consumer in lock-step regardless of which scope is
    /// constructing the closure.
    pub(crate) fn wrapper_fn_ptr_capture_index(
        &self,
        func_id: FuncId,
        hir_module: &hir::Module,
    ) -> Option<usize> {
        if !self.is_wrapper_func(&func_id) {
            return None;
        }
        let func = hir_module.func_defs.get(&func_id)?;
        let known_param_name = self.closures.wrapper_func_param_name.get(&func_id).copied();
        for (i, param) in func.params.iter().enumerate() {
            let param_name = self.interner.resolve(param.name);
            let matches = if let Some(known) = known_param_name {
                let known_str = self.interner.resolve(known);
                let capture_variant = format!("__capture_{}", known_str);
                param_name == known_str || param_name == capture_variant.as_str()
            } else {
                // Same fallback as `function_lowering.rs::insert_func_ptr_param`
                // for wrappers not covered by the pre-scan.
                param_name == "func" || param_name == "__capture_func"
            };
            if matches {
                return Some(i);
            }
        }
        None
    }
}

// =============================================================================
// Class Info (classes.*)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Get class info by ClassId.
    pub(crate) fn get_class_info(&self, class_id: &ClassId) -> Option<&LoweredClassInfo> {
        self.classes.class_info.get(class_id)
    }

    /// Insert class info for a ClassId.
    pub(crate) fn insert_class_info(&mut self, class_id: ClassId, info: LoweredClassInfo) {
        self.classes.class_info.insert(class_id, info);
    }

    /// Check if a class exists.
    pub(crate) fn has_class(&self, class_id: &ClassId) -> bool {
        self.classes.class_info.contains_key(class_id)
    }

    /// Get ClassId by class name.
    pub(crate) fn get_class_by_name(&self, name: &str) -> Option<ClassId> {
        self.classes.class_name_map.get(name).copied()
    }

    /// Register a class name to ClassId mapping.
    pub(crate) fn register_class_name(&mut self, name: String, class_id: ClassId) {
        self.classes.class_name_map.insert(name, class_id);
    }

    /// Iterate over all class info entries.
    pub(crate) fn class_info_iter(&self) -> impl Iterator<Item = (&ClassId, &LoweredClassInfo)> {
        self.classes.class_info.iter()
    }

    /// Return `true` iff `child` is a STRICT (proper) subclass of `parent` —
    /// they are not the same class and `parent` appears anywhere on
    /// `child`'s base-class chain. Used for the CPython §3.3.8
    /// subclass-first rule in operator dunder dispatch.
    pub(crate) fn is_proper_subclass(&self, child: ClassId, parent: ClassId) -> bool {
        if child == parent {
            return false;
        }
        let mut current = child;
        while let Some(info) = self.get_class_info(&current) {
            match info.base_class {
                Some(base) if base == parent => return true,
                Some(base) => current = base,
                None => return false,
            }
        }
        false
    }
}

// =============================================================================
// Global Variables (symbols.globals)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Check if a variable is a global.
    pub(crate) fn is_global(&self, var_id: &VarId) -> bool {
        self.symbols.globals.contains(var_id)
    }
}

// =============================================================================
// Cell Variables (symbols.cell_vars, symbols.nonlocal_cells)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Check if a variable is a cell variable.
    pub(crate) fn is_cell_var(&self, var_id: &VarId) -> bool {
        self.symbols.cell_vars.contains(var_id)
    }

    /// Get the cell local for a nonlocal variable.
    pub(crate) fn get_nonlocal_cell(&self, var_id: &VarId) -> Option<LocalId> {
        self.symbols.nonlocal_cells.get(var_id).copied()
    }

    /// Map a nonlocal variable to its cell local.
    pub(crate) fn insert_nonlocal_cell(&mut self, var_id: VarId, local_id: LocalId) {
        self.symbols.nonlocal_cells.insert(var_id, local_id);
    }

    /// Check if a variable has a nonlocal cell mapping.
    pub(crate) fn has_nonlocal_cell(&self, var_id: &VarId) -> bool {
        self.symbols.nonlocal_cells.contains_key(var_id)
    }

    /// Clone the nonlocal cells mapping (for saving/restoring state).
    pub(crate) fn clone_nonlocal_cells(&self) -> IndexMap<VarId, LocalId> {
        self.symbols.nonlocal_cells.clone()
    }

    /// Restore nonlocal cells from a saved state.
    pub(crate) fn restore_nonlocal_cells(&mut self, cells: IndexMap<VarId, LocalId>) {
        self.symbols.nonlocal_cells = cells;
    }
}

// =============================================================================
// Function Return Types (func_return_types, symbols.current_func_return_type)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Get the return type for a function.
    pub(crate) fn get_func_return_type(&self, func_id: &FuncId) -> Option<&Type> {
        self.func_return_types.inner.get(func_id)
    }

    /// Set the return type for a function.
    pub(crate) fn insert_func_return_type(&mut self, func_id: FuncId, ty: Type) {
        self.func_return_types.inner.insert(func_id, ty);
    }

    /// Get the current function's return type.
    pub(crate) fn get_current_func_return_type(&self) -> Option<&Type> {
        self.symbols.current_func_return_type.as_ref()
    }
}

// =============================================================================
// Closure Capture Types (closures.closure_capture_types)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Get closure capture types for a function.
    pub(crate) fn get_closure_capture_types(&self, func_id: &FuncId) -> Option<&Vec<Type>> {
        self.closures.closure_capture_types.get(func_id)
    }

    /// Set closure capture types for a function.
    pub(crate) fn insert_closure_capture_types(&mut self, func_id: FuncId, types: Vec<Type>) {
        self.closures.closure_capture_types.insert(func_id, types);
    }

    /// Check if closure capture types are tracked for a function.
    pub(crate) fn has_closure_capture_types(&self, func_id: &FuncId) -> bool {
        self.closures.closure_capture_types.contains_key(func_id)
    }
}

// =============================================================================
// Lambda Parameter Type Hints (closures.lambda_param_type_hints)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Get caller-provided parameter type hints for a lambda.
    pub(crate) fn get_lambda_param_type_hints(&self, func_id: &FuncId) -> Option<&Vec<Type>> {
        self.closures.lambda_param_type_hints.get(func_id)
    }

    /// Set parameter type hints for a lambda.
    pub(crate) fn insert_lambda_param_type_hints(&mut self, func_id: FuncId, types: Vec<Type>) {
        self.closures.lambda_param_type_hints.insert(func_id, types);
    }
}

// =============================================================================
// Module Exports (modules.*)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Get a module variable export.
    pub(crate) fn get_module_var_export(&self, key: &(String, String)) -> Option<&(VarId, Type)> {
        self.modules.module_var_exports.get(key)
    }

    /// Get a module function export (return type).
    pub(crate) fn get_module_func_export(&self, key: &(String, String)) -> Option<&Type> {
        self.modules.module_func_exports.get(key)
    }

    /// Get a module function's parameter list (cross-module kwargs / defaults).
    pub(crate) fn get_module_func_params(
        &self,
        key: &(String, String),
    ) -> Option<&Vec<super::ExportedParam>> {
        self.modules.module_func_params.get(key)
    }

    /// Get a module class export (ClassId, class_name).
    pub(crate) fn get_module_class_export(
        &self,
        key: &(String, String),
    ) -> Option<&(ClassId, String)> {
        self.modules.module_class_exports.get(key)
    }

    /// Iterate over all module class exports.
    pub(crate) fn module_class_exports_iter(
        &self,
    ) -> impl Iterator<Item = (&(String, String), &(ClassId, String))> {
        self.modules.module_class_exports.iter()
    }

    /// Get cross-module class info.
    pub(crate) fn get_cross_module_class_info(
        &self,
        class_id: &ClassId,
    ) -> Option<&CrossModuleClassInfo> {
        self.modules.cross_module_class_info.get(class_id)
    }
}

// =============================================================================
// Default Value Slots (symbols.default_value_slots)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Get the global slot for a mutable default parameter.
    pub(crate) fn get_default_slot(&self, key: &(FuncId, usize)) -> Option<u32> {
        self.symbols.default_value_slots.get(key).copied()
    }
}

// =============================================================================
// Pending Varargs/Kwargs (codegen.*)
// =============================================================================

impl<'a> Lowering<'a> {
    /// Set the pending varargs tuple from list unpacking.
    pub(crate) fn set_pending_varargs(&mut self, local_id: LocalId) {
        self.codegen.pending_varargs_from_unpack = Some(local_id);
    }

    /// Take the pending varargs tuple.
    pub(crate) fn take_pending_varargs(&mut self) -> Option<LocalId> {
        self.codegen.pending_varargs_from_unpack.take()
    }

    /// Set the pending kwargs dict from **kwargs unpacking.
    pub(crate) fn set_pending_kwargs(&mut self, local_id: LocalId, value_type: Type) {
        self.codegen.pending_kwargs_from_unpack = Some((local_id, value_type));
    }

    /// Take the pending kwargs dict.
    pub(crate) fn take_pending_kwargs(&mut self) -> Option<(LocalId, Type)> {
        self.codegen.pending_kwargs_from_unpack.take()
    }

    /// Clear the pending kwargs without taking.
    pub(crate) fn clear_pending_kwargs(&mut self) {
        self.codegen.pending_kwargs_from_unpack = None;
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
    #[allow(dead_code)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use pyaot_hir as hir;
    use pyaot_mir as mir;
    use pyaot_utils::{FuncId, Span, StringInterner};

    fn expr(kind: hir::ExprKind) -> hir::Expr {
        hir::Expr {
            kind,
            ty: None,
            span: Span::dummy(),
        }
    }

    #[test]
    fn seed_expr_type_does_not_recursively_infer_unannotated_binop() {
        let mut interner = StringInterner::default();
        let mut lowering = Lowering::new(&mut interner);
        let module_name = lowering.intern("seed_expr_type_test");
        let mut hir_module = hir::Module::new(module_name);

        let left = hir_module.exprs.alloc(expr(hir::ExprKind::Int(1)));
        let right = hir_module.exprs.alloc(expr(hir::ExprKind::Float(2.0)));
        let bin = hir_module.exprs.alloc(expr(hir::ExprKind::BinOp {
            op: hir::BinOp::Add,
            left,
            right,
        }));

        assert_eq!(lowering.seed_expr_type(bin, &hir_module), Type::Any);
    }

    #[test]
    fn seed_expr_type_prefers_cached_seed_metadata_for_non_var_expressions() {
        let mut interner = StringInterner::default();
        let mut lowering = Lowering::new(&mut interner);
        let module_name = lowering.intern("seed_expr_type_cache_test");
        let mut hir_module = hir::Module::new(module_name);

        let left = hir_module.exprs.alloc(expr(hir::ExprKind::Int(1)));
        let right = hir_module.exprs.alloc(expr(hir::ExprKind::Int(2)));
        let bin = hir_module.exprs.alloc(expr(hir::ExprKind::BinOp {
            op: hir::BinOp::Add,
            left,
            right,
        }));
        lowering.lowering_seed_info.insert_type(bin, Type::Int);

        assert_eq!(lowering.seed_expr_type(bin, &hir_module), Type::Int);
    }

    #[test]
    fn resolved_value_type_hint_prefers_lowered_operand_type_over_seed_any() {
        let mut interner = StringInterner::default();
        let mut lowering = Lowering::new(&mut interner);
        let module_name = lowering.intern("resolved_value_type_hint_test");
        let mut hir_module = hir::Module::new(module_name);

        let left = hir_module.exprs.alloc(expr(hir::ExprKind::Int(1)));
        let right = hir_module.exprs.alloc(expr(hir::ExprKind::Int(2)));
        let bin = hir_module.exprs.alloc(expr(hir::ExprKind::BinOp {
            op: hir::BinOp::Add,
            left,
            right,
        }));

        let func_id = FuncId::from(0u32);
        let local_id = LocalId::from(0u32);
        let mut mir_func =
            mir::Function::new(func_id, "test".to_string(), Vec::new(), Type::None, None);
        mir_func.add_local(mir::Local {
            id: local_id,
            name: None,
            ty: Type::Int,
            is_gc_root: false,
        });

        assert_eq!(
            lowering.resolved_value_type_hint(
                bin,
                &mir::Operand::Local(local_id),
                &hir_module,
                &mir_func,
            ),
            Type::Int
        );
    }
}
