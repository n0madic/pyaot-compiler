//! Accessor methods for Lowering context internal state
//!
//! These methods provide controlled access to the Lowering context's internal state.
//! They encapsulate common access patterns and reduce tight coupling between modules.

use indexmap::IndexMap;
use pyaot_diagnostics::CompilerWarning;
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::{typespec_to_type, Type};
use pyaot_utils::{BlockId, ClassId, FuncId, InternedString, LocalId, VarId};

use crate::narrowing::DeadBranch;
use crate::type_planning::helpers;

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
    /// at a use site must go through `expr_type_hint` — its Var
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

    /// Lightweight lowering-time type hint.
    ///
    /// This is intentionally not a recursive HIR inference engine. It reads the
    /// current lowered view for `Var` expressions and otherwise falls back to the
    /// HIR node's own annotation. Seed-building passes inside `type_planning`
    /// still use their private inference helpers; regular lowering must not.
    pub(crate) fn expr_type_hint(&self, expr_id: hir::ExprId, hir_module: &hir::Module) -> Type {
        let expr = &hir_module.exprs[expr_id];
        if !matches!(expr.kind, hir::ExprKind::Var(_)) {
            if let Some(cached) = self.lowering_seed_info.lookup(expr_id).cloned() {
                return cached;
            }
        }
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
            hir::ExprKind::BinOp { op, left, right } => {
                if let Some(annotated) = expr.ty.clone() {
                    return annotated;
                }
                let left_ty = self.expr_type_hint(*left, hir_module);
                let right_ty = self.expr_type_hint(*right, hir_module);
                match op {
                    hir::BinOp::Add => match (&left_ty, &right_ty) {
                        (Type::Float, _) | (_, Type::Float) => Type::Float,
                        (Type::Int, Type::Int) | (Type::Bool, Type::Bool) => Type::Int,
                        (Type::Str, Type::Str) => Type::Str,
                        (Type::Bytes, Type::Bytes) => Type::Bytes,
                        (Type::List(left), Type::List(right)) => {
                            Type::List(Box::new(Type::unify_field_type(left, right)))
                        }
                        (Type::Tuple(left), Type::Tuple(right)) => {
                            let mut elems = left.clone();
                            elems.extend(right.clone());
                            Type::Tuple(elems)
                        }
                        _ => Type::Any,
                    },
                    hir::BinOp::Sub
                    | hir::BinOp::Mul
                    | hir::BinOp::Div
                    | hir::BinOp::FloorDiv
                    | hir::BinOp::Mod
                    | hir::BinOp::Pow => {
                        if matches!(left_ty, Type::Float) || matches!(right_ty, Type::Float) {
                            Type::Float
                        } else if matches!(left_ty, Type::Int | Type::Bool)
                            && matches!(right_ty, Type::Int | Type::Bool)
                        {
                            Type::Int
                        } else {
                            Type::Any
                        }
                    }
                    hir::BinOp::BitAnd
                    | hir::BinOp::BitOr
                    | hir::BinOp::BitXor
                    | hir::BinOp::LShift
                    | hir::BinOp::RShift => Type::Int,
                    hir::BinOp::MatMul => Type::Any,
                }
            }
            hir::ExprKind::UnOp { op, operand } => match op {
                hir::UnOp::Not => Type::Bool,
                hir::UnOp::Neg | hir::UnOp::Pos => self.expr_type_hint(*operand, hir_module),
                hir::UnOp::Invert => Type::Int,
            },
            hir::ExprKind::Compare { .. } => Type::Bool,
            hir::ExprKind::LogicalOp { left, right, .. } => {
                if let Some(annotated) = expr.ty.clone() {
                    return annotated;
                }
                let left_ty = self.expr_type_hint(*left, hir_module);
                let right_ty = self.expr_type_hint(*right, hir_module);
                if left_ty == right_ty {
                    left_ty
                } else {
                    Type::normalize_union(vec![left_ty, right_ty])
                }
            }
            hir::ExprKind::IfExpr {
                then_val, else_val, ..
            } => {
                if let Some(annotated) = expr.ty.clone() {
                    return annotated;
                }
                let then_ty = self.expr_type_hint(*then_val, hir_module);
                let else_ty = self.expr_type_hint(*else_val, hir_module);
                if then_ty == else_ty {
                    then_ty
                } else {
                    Type::normalize_union(vec![then_ty, else_ty])
                }
            }
            hir::ExprKind::List(elements) => {
                if let Some(annotated) = expr.ty.clone() {
                    return annotated;
                }
                let elem_ty = elements.iter().fold(None, |acc: Option<Type>, elem_id| {
                    let next = self.expr_type_hint(*elem_id, hir_module);
                    Some(match acc {
                        Some(prev) => Type::unify_field_type(&prev, &next),
                        None => next,
                    })
                });
                Type::List(Box::new(elem_ty.unwrap_or(Type::Any)))
            }
            hir::ExprKind::Tuple(elements) => {
                if let Some(annotated) = expr.ty.clone() {
                    return annotated;
                }
                Type::Tuple(
                    elements
                        .iter()
                        .map(|elem_id| self.expr_type_hint(*elem_id, hir_module))
                        .collect(),
                )
            }
            hir::ExprKind::Set(elements) => {
                if let Some(annotated) = expr.ty.clone() {
                    return annotated;
                }
                let elem_ty = elements.iter().fold(None, |acc: Option<Type>, elem_id| {
                    let next = self.expr_type_hint(*elem_id, hir_module);
                    Some(match acc {
                        Some(prev) => Type::unify_field_type(&prev, &next),
                        None => next,
                    })
                });
                Type::Set(Box::new(elem_ty.unwrap_or(Type::Any)))
            }
            hir::ExprKind::Dict(pairs) => {
                if let Some(annotated) = expr.ty.clone() {
                    return annotated;
                }
                let (key_ty, value_ty) = pairs.iter().fold(
                    (None, None),
                    |(acc_k, acc_v): (Option<Type>, Option<Type>), (key_id, value_id)| {
                        let next_k = self.expr_type_hint(*key_id, hir_module);
                        let next_v = self.expr_type_hint(*value_id, hir_module);
                        (
                            Some(match acc_k {
                                Some(prev) => Type::unify_field_type(&prev, &next_k),
                                None => next_k,
                            }),
                            Some(match acc_v {
                                Some(prev) => Type::unify_field_type(&prev, &next_v),
                                None => next_v,
                            }),
                        )
                    },
                );
                Type::Dict(
                    Box::new(key_ty.unwrap_or(Type::Any)),
                    Box::new(value_ty.unwrap_or(Type::Any)),
                )
            }
            hir::ExprKind::Call { func, .. } => {
                let func_expr = &hir_module.exprs[*func];
                match &func_expr.kind {
                    hir::ExprKind::FuncRef(func_id) => self
                        .get_func_return_type(func_id)
                        .cloned()
                        .or_else(|| {
                            hir_module
                                .func_defs
                                .get(func_id)
                                .and_then(|f| f.return_type.clone())
                        })
                        .or_else(|| expr.ty.clone())
                        .unwrap_or(Type::Any),
                    hir::ExprKind::Var(var_id) => self
                        .get_var_func(var_id)
                        .and_then(|func_id| self.get_func_return_type(&func_id).cloned())
                        .or_else(|| expr.ty.clone())
                        .unwrap_or(Type::Any),
                    _ => expr.ty.clone().unwrap_or(Type::Any),
                }
            }
            hir::ExprKind::BuiltinCall { .. } => expr.ty.clone().unwrap_or(Type::Any),
            hir::ExprKind::MethodCall { obj, method, .. } => {
                let obj_ty = self.expr_type_hint(*obj, hir_module);
                let method_name = self.resolve(*method);
                if let Some(ret_ty) = helpers::resolve_method_return_type(&obj_ty, method_name) {
                    return ret_ty;
                }
                if let Type::Class { class_id, .. } = obj_ty {
                    if let Some(class_info) = self.get_class_info(&class_id) {
                        for methods in [
                            &class_info.method_funcs,
                            &class_info.class_methods,
                            &class_info.static_methods,
                        ] {
                            if let Some(&func_id) = methods.get(method) {
                                if let Some(ret_ty) = self.get_func_return_type(&func_id) {
                                    return ret_ty.clone();
                                }
                                if let Some(func_def) = hir_module.func_defs.get(&func_id) {
                                    return func_def.return_type.clone().unwrap_or(Type::Any);
                                }
                            }
                        }
                        if let Some(func_id) = class_info.get_dunder_func(method_name) {
                            if let Some(ret_ty) = self.get_func_return_type(&func_id) {
                                return ret_ty.clone();
                            }
                        }
                    }
                }
                expr.ty.clone().unwrap_or(Type::Any)
            }
            hir::ExprKind::Attribute { obj, attr } => {
                let obj_ty = self.expr_type_hint(*obj, hir_module);
                if let Type::Class { class_id, .. } = obj_ty {
                    if let Some(class_info) = self.get_class_info(&class_id) {
                        if let Some(field_ty) = class_info.field_types.get(attr) {
                            return field_ty.clone();
                        }
                        if let Some(prop_ty) = class_info.property_types.get(attr) {
                            return prop_ty.clone();
                        }
                        if let Some(class_attr_ty) = class_info.class_attr_types.get(attr) {
                            return class_attr_ty.clone();
                        }
                    }
                }
                expr.ty.clone().unwrap_or(Type::Any)
            }
            hir::ExprKind::Index { obj, .. } => {
                let obj_ty = self.expr_type_hint(*obj, hir_module);
                match obj_ty {
                    Type::List(elem) | Type::Set(elem) | Type::Iterator(elem) => *elem,
                    Type::Tuple(items) => items.first().cloned().unwrap_or(Type::Any),
                    Type::TupleVar(elem) => *elem,
                    Type::Dict(_, value) | Type::DefaultDict(_, value) => *value,
                    Type::Str => Type::Str,
                    Type::Bytes => Type::Int,
                    _ => expr.ty.clone().unwrap_or(Type::Any),
                }
            }
            hir::ExprKind::Slice { obj, .. } => {
                let obj_ty = self.expr_type_hint(*obj, hir_module);
                if matches!(obj_ty, Type::Any) {
                    expr.ty.clone().unwrap_or(Type::Any)
                } else {
                    obj_ty
                }
            }
            hir::ExprKind::StdlibCall { func, args } => {
                let declared = typespec_to_type(&func.return_type);
                if !matches!(declared, Type::Any | Type::HeapAny) {
                    declared
                } else if let Some(annotated) = expr.ty.clone() {
                    if !matches!(annotated, Type::Any | Type::HeapAny) {
                        annotated
                    } else if let Some(expected) = self.codegen.expected_type.clone() {
                        if !matches!(expected, Type::Any | Type::HeapAny) {
                            expected
                        } else {
                            match func.name {
                                "choice" => args
                                    .first()
                                    .map(|arg_id| {
                                        crate::type_planning::infer::extract_iterable_first_element_type(
                                            &self.expr_type_hint(*arg_id, hir_module),
                                        )
                                    })
                                    .unwrap_or(Type::Any),
                                "sample" | "choices" => args
                                    .first()
                                    .map(|arg_id| {
                                        Type::List(Box::new(
                                            crate::type_planning::infer::extract_iterable_first_element_type(
                                                &self.expr_type_hint(*arg_id, hir_module),
                                            ),
                                        ))
                                    })
                                    .unwrap_or(Type::Any),
                                _ => annotated,
                            }
                        }
                    } else {
                        annotated
                    }
                } else {
                    match func.name {
                        "choice" => args
                            .first()
                            .map(|arg_id| {
                                crate::type_planning::infer::extract_iterable_first_element_type(
                                    &self.expr_type_hint(*arg_id, hir_module),
                                )
                            })
                            .unwrap_or(Type::Any),
                        "sample" | "choices" => args
                            .first()
                            .map(|arg_id| {
                                Type::List(Box::new(
                                    crate::type_planning::infer::extract_iterable_first_element_type(
                                        &self.expr_type_hint(*arg_id, hir_module),
                                    ),
                                ))
                            })
                            .unwrap_or(Type::Any),
                        _ => declared,
                    }
                }
            }
            hir::ExprKind::StdlibAttr(attr_def) => typespec_to_type(&attr_def.ty),
            hir::ExprKind::StdlibConst(const_def) => typespec_to_type(&const_def.ty),
            _ => expr.ty.clone().unwrap_or(Type::Any),
        }
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
