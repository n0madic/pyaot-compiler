//! Infer mode: bottom-up type synthesis
//!
//! # Phase 1 §1.4 migration surface (updated 2026-04-18 for §1.4u)
//!
//! **Two public HIR type-query entry points.** Consumers outside
//! `type_planning/` must route through exactly one of these:
//!
//! - [`Lowering::seed_expr_type_by_id`] — the memoized lowering-time
//!   query. Takes an `ExprId`, caches per-expression results. Nearly
//!   every post-type-planning caller in `statements/`, `expressions/`,
//!   `exceptions.rs`, etc. funnels through this path (~124 call
//!   sites per the §1.4u caller audit).
//! - [`Lowering::seed_infer_expr_type`] — the pre-scan / non-memoized
//!   path. Takes an `&hir::Expr` and a `param_types` overlay (pass
//!   `&IndexMap::new()` for no overlay). Used by the 10 prescan
//!   walkers in `type_planning/*` that need to query types before the
//!   memoization cache is populated.
//!
//! Nothing outside `type_planning/` should call `compute_expr_type` or
//! `infer_expr_type_inner` directly — these are `pub(super)`
//! implementation details. Full unification into a single unified
//! match (the spec's §1.4u goal) is blocked on the borrow-checker
//! issue documented below; the two wrappers share 11 `resolve_*`
//! helpers for complex arms (Method/Call/Builtin/Attribute/Index/
//! Class/Closure/Module), so the real duplication is limited to the
//! per-arm sub-expression resolution dispatch.
//!
//! # Internal structure (deleted in S1.9b)
//!
//! - `compute_expr_type` — codegen path (`&mut self`), recurses via
//!   `seed_expr_type_by_id` for memoized sub-expression resolution.
//! - `infer_expr_type_inner` — pre-scan path (`&self`), recurses directly
//!   without memoization. Used by `seed_infer_expr_type` for return type
//!   inference and lambda/closure analysis before codegen starts.
//!
//! Complex match arms (MethodCall, Call, BuiltinCall, Attribute, Index)
//! are factored into shared `resolve_*` helpers that both paths call
//! after resolving sub-expression types.
//!
//! # Why two functions instead of one
//!
//! `compute_seed_expr_type` MUST recurse through `seed_expr_type_by_id` (which
//! caches results in `expr_types`). During lowering, `var_types` evolves
//! as statements are processed. If sub-expression types are computed fresh
//! (without cache), the same expression can produce different types at
//! different points in time — e.g., a variable starts as `Any` before
//! assignment, then becomes `Str` after. The cache freezes the type at
//! first computation, ensuring consistent codegen.
//!
//! `infer_seed_expr_type_inner` takes `&self` and cannot call `seed_expr_type_by_id`
//! (which requires `&mut self`). It also MUST NOT cache into `expr_types`
//! because it runs during pre-scan before lowering — caching at that point
//! would freeze stale types that the codegen path would later pick up.
//!
//! A closure-based unification also fails: a shared `unified(&self, ..., F)`
//! cannot accept a closure `|id| self.seed_expr_type_by_id(id)` because the
//! closure needs `&mut self` while `unified` already holds `&self`.
//!
//! # Adding new match arms
//!
//! When adding support for a new `ExprKind` variant:
//! 1. Add the match arm to BOTH `compute_expr_type` and `infer_expr_type_inner`.
//! 2. If the arm has complex logic (>5 lines) that doesn't depend on how
//!    sub-expressions are resolved, extract it into a `resolve_*` helper
//!    method (takes `&self`, receives pre-computed types as arguments).
//! 3. Do NOT add explicit literal arms (Int/Float/Bool/Str/Bytes/None) to
//!    `compute_expr_type` — the codegen path relies on `expr.ty` fallback
//!    for literals to maintain consistency with how `var_types` caching
//!    interacts with type resolution order.

use indexmap::IndexMap;
use pyaot_hir as hir;
use pyaot_stdlib_defs::{lookup_object_field, lookup_object_type};
use pyaot_types::{typespec_to_type, Type, TypeLattice};
use pyaot_utils::interner::InternedString;
use pyaot_utils::VarId;

use super::helpers;
use crate::context::Lowering;

// =============================================================================
// Shared helpers: type resolution after sub-expressions are computed.
// All take `&self` so both `&mut self` (codegen) and `&self` (pre-scan) can use them.
// =============================================================================

impl<'a> Lowering<'a> {
    /// Resolve method call type on an already-resolved object type.
    /// Returns `None` if no resolution found (caller applies fallback).
    fn resolve_method_on_type(
        &self,
        obj_ty: &Type,
        method: InternedString,
        method_name: &str,
        module: &hir::Module,
    ) -> Option<Type> {
        // Shared dispatch table (Str, List, Dict, Set, File)
        if let Some(ty) = helpers::resolve_method_return_type(obj_ty, method_name) {
            return Some(ty);
        }
        match obj_ty {
            Type::Class { ref class_id, .. } => {
                if let Some(class_info) = self.get_class_info(class_id) {
                    let method_maps = [
                        &class_info.method_funcs,
                        &class_info.class_methods,
                        &class_info.static_methods,
                    ];
                    for methods in method_maps {
                        if let Some(&method_func_id) = methods.get(&method) {
                            if let Some(ret_ty) = self.get_func_return_type(&method_func_id) {
                                return Some(ret_ty.clone());
                            }
                            if let Some(func_def) = module.func_defs.get(&method_func_id) {
                                return Some(func_def.return_type.clone().unwrap_or(Type::None));
                            }
                        }
                    }
                    if let Some(func_id) = class_info.get_dunder_func(method_name) {
                        if let Some(ret_ty) = self.get_func_return_type(&func_id) {
                            return Some(ret_ty.clone());
                        }
                        if let Some(func_def) = module.func_defs.get(&func_id) {
                            if let Some(ret_ty) = func_def.return_type.clone() {
                                return Some(ret_ty);
                            }
                        }
                        return Some(match method_name {
                            "__eq__" | "__ne__" | "__lt__" | "__le__" | "__gt__" | "__ge__"
                            | "__bool__" | "__contains__" => Type::Bool,
                            "__str__" | "__repr__" => Type::Str,
                            "__hash__" | "__len__" => Type::Int,
                            "__setitem__" | "__delitem__" => Type::None,
                            _ => obj_ty.clone(),
                        });
                    }
                }
                // Fall through to cross-module info (classes imported from
                // another module — local `class_info` never had them).
                if let Some(ret_ty) = self
                    .get_cross_module_class_info(class_id)
                    .and_then(|info| info.method_return_types.get(&method))
                    .cloned()
                {
                    return Some(ret_ty);
                }
                None
            }
            Type::RuntimeObject(type_tag) => {
                if let Some(obj_def) = lookup_object_type(*type_tag) {
                    if let Some(method_def) = obj_def.get_method(method_name) {
                        return Some(typespec_to_type(&method_def.return_type));
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// When at least one operand of a binary op is a user class, look up
    /// the dispatched dunder's *actual* inferred return type. Returns
    /// `None` if neither operand is a user class or the corresponding
    /// dunder is missing — in which case the caller falls back to the
    /// structural `resolve_binop_type` heuristic.
    ///
    /// Mirrors the dispatch order in `lower_binop`: subclass-first,
    /// forward, reverse — so the inferred type matches the actually
    /// dispatched dunder.
    pub(crate) fn resolve_class_binop_return(
        &self,
        op: &hir::BinOp,
        left_ty: &Type,
        right_ty: &Type,
    ) -> Option<Type> {
        let forward = op.forward_dunder();
        let reflected = op.reflected_dunder();

        // Subclass-first: right is a strict subclass of left and has reflected.
        if let (Type::Class { class_id: l_id, .. }, Type::Class { class_id: r_id, .. }) =
            (left_ty, right_ty)
        {
            if l_id != r_id && self.is_proper_subclass(*r_id, *l_id) {
                if let Some(rfid) = self
                    .get_class_info(r_id)
                    .and_then(|ci| ci.get_dunder_func(reflected))
                {
                    if let Some(rt) = self.get_func_return_type(&rfid) {
                        return Some(rt.clone());
                    }
                }
            }
        }
        // Forward dunder on left.
        let forward_ret = if let Type::Class { class_id, .. } = left_ty {
            self.get_class_info(class_id)
                .and_then(|ci| ci.get_dunder_func(forward))
                .and_then(|fid| self.get_func_return_type(&fid).cloned())
        } else {
            None
        };
        // Reflected dunder on right (used both for the no-forward fallback
        // and for unioning into the forward's return when forward may
        // return NotImplemented).
        let reflected_ret = if let Type::Class { class_id, .. } = right_ty {
            self.get_class_info(class_id)
                .and_then(|ci| ci.get_dunder_func(reflected))
                .and_then(|rfid| self.get_func_return_type(&rfid).cloned())
        } else {
            None
        };

        // After the §3.3.8 NotImplemented fallback, the call-site value is
        // (forward's returns \ NotImplementedT) ∪ reflected's returns. So
        // strip NotImplementedT from the forward type and union with
        // reflected — that's what the consumer of the binop actually sees.
        fn strip_ni(ty: Type) -> Option<Type> {
            match ty {
                Type::NotImplementedT => None,
                Type::Union(members) => {
                    let kept: Vec<Type> = members
                        .into_iter()
                        .filter(|t| *t != Type::NotImplementedT)
                        .collect();
                    if kept.is_empty() {
                        None
                    } else {
                        Some(
                            kept.into_iter()
                                .reduce(|a, b| a.join(&b))
                                .unwrap_or(Type::Never),
                        )
                    }
                }
                other => Some(other),
            }
        }
        let forward_visible = forward_ret.and_then(strip_ni);
        match (forward_visible, reflected_ret) {
            (Some(f), Some(r)) if f != r => Some(f.join(&r)),
            (Some(f), _) => Some(f),
            (None, Some(r)) => Some(r),
            (None, None) => None,
        }
    }

    /// Resolve call target type from the function expression.
    /// Returns `None` if no resolution found (caller applies fallback).
    fn resolve_call_target_type(
        &self,
        func_expr: &hir::Expr,
        module: &hir::Module,
    ) -> Option<Type> {
        if let hir::ExprKind::FuncRef(func_id) = &func_expr.kind {
            if let Some(return_type) = self.get_func_return_type(func_id) {
                return Some(return_type.clone());
            }
            if let Some(func_def) = module.func_defs.get(func_id) {
                return Some(func_def.return_type.clone().unwrap_or(Type::None));
            }
        }
        // Immediate Closure call, e.g. `(Closure { __genexp_N, [captures] })()`
        // emitted by gen-expr desugaring. Return the wrapped function's return
        // type so downstream `sum`/`min`/`max` dispatch sees `Iterator(...)`.
        if let hir::ExprKind::Closure { func: func_id, .. } = &func_expr.kind {
            if let Some(return_type) = self.get_func_return_type(func_id) {
                return Some(return_type.clone());
            }
            if let Some(func_def) = module.func_defs.get(func_id) {
                return Some(func_def.return_type.clone().unwrap_or(Type::None));
            }
        }
        if let hir::ExprKind::Var(var_id) = &func_expr.kind {
            // §1.4u-b: base var type; narrowing never upgrades a variable
            // to `Type::Class` — a `Var` with a class type is the declared
            // or prescan-inferred type of that variable.
            if let Some(Type::Class { class_id, .. }) =
                self.get_base_var_type(var_id).cloned().as_ref()
            {
                if let Some(call_func_id) = self
                    .get_class_info(class_id)
                    .and_then(|info| info.get_dunder_func("__call__"))
                {
                    if let Some(return_type) = self.get_func_return_type(&call_func_id) {
                        return Some(return_type.clone());
                    }
                    if let Some(func_def) = module.func_defs.get(&call_func_id) {
                        return Some(func_def.return_type.clone().unwrap_or(Type::None));
                    }
                }
            }
            if self.is_func_ptr_param(var_id) {
                if let Some(return_type) = self.get_current_func_return_type() {
                    return Some(return_type.clone());
                }
                return Some(Type::Any);
            }
            if let Some((_, original_func_id)) = self.get_var_wrapper(var_id) {
                if let Some(return_type) = self.get_func_return_type(&original_func_id) {
                    return Some(return_type.clone());
                }
                if let Some(func_def) = module.func_defs.get(&original_func_id) {
                    return Some(func_def.return_type.clone().unwrap_or(Type::None));
                }
            }
            if let Some((_, original_func_id)) = self.get_module_var_wrapper(var_id) {
                if let Some(return_type) = self.get_func_return_type(&original_func_id) {
                    return Some(return_type.clone());
                }
                if let Some(func_def) = module.func_defs.get(&original_func_id) {
                    return Some(func_def.return_type.clone().unwrap_or(Type::None));
                }
            }
            if let Some(func_id) = self.get_var_func(var_id) {
                if let Some(return_type) = self.get_func_return_type(&func_id) {
                    return Some(return_type.clone());
                }
                if let Some(func_def) = module.func_defs.get(&func_id) {
                    return Some(func_def.return_type.clone().unwrap_or(Type::None));
                }
            }
            // Identity-decorated module-level functions: `@identity def f(): …`
            // leaves `f` as a Var pointing at the original FuncId (tracked in
            // `module_var_funcs`, populated by `process_module_decorated_functions`).
            // Required so eager-cache of Call-expr return types sees the original
            // function's return type instead of falling through to `Type::Any`.
            if let Some(func_id) = self.get_module_var_func(var_id) {
                if let Some(return_type) = self.get_func_return_type(&func_id) {
                    return Some(return_type.clone());
                }
                if let Some(func_def) = module.func_defs.get(&func_id) {
                    return Some(func_def.return_type.clone().unwrap_or(Type::None));
                }
            }
        }
        if let hir::ExprKind::ClassRef(class_id) = &func_expr.kind {
            if let Some(class_def) = module.class_defs.get(class_id) {
                return Some(Type::Class {
                    class_id: *class_id,
                    name: class_def.name,
                });
            }
        }
        if let hir::ExprKind::ModuleAttr {
            module: mod_name,
            attr,
        } = &func_expr.kind
        {
            let attr_name = self.resolve(*attr).to_string();
            let key = (mod_name.clone(), attr_name);
            if let Some((class_id, _)) = self.get_module_class_export(&key) {
                return Some(Type::Class {
                    class_id: *class_id,
                    name: *attr,
                });
            }
            // A `pkg.func()` call reaches here as a `ModuleAttr` func_expr,
            // not as `ImportedRef` — propagate the function's return type
            // the same way as the `ImportedRef` branch below.
            if let Some(return_type) = self.get_module_func_export(&key) {
                return Some(return_type.clone());
            }
        }
        if let hir::ExprKind::ImportedRef {
            module: mod_name,
            name,
        } = &func_expr.kind
        {
            let key = (mod_name.clone(), name.clone());
            // Class constructor: `from mymod import Foo; Foo(...)` lowers to
            // a `Call { func: ImportedRef, ... }`. The call expression's
            // type is the class being instantiated — use module_class_exports
            // to recover the remapped class id. The class name must already
            // be interned in the caller (it appeared as an import alias);
            // skip the shortcut otherwise and fall through to the function-
            // export lookup so we don't fabricate a garbage `InternedString`.
            if let Some((class_id, class_name)) = self.get_module_class_export(&key).cloned() {
                if let Some(name_interned) = self.lookup_interned(&class_name) {
                    return Some(Type::Class {
                        class_id,
                        name: name_interned,
                    });
                }
            }
            if let Some(return_type) = self.get_module_func_export(&key) {
                return Some(return_type.clone());
            }
        }
        None
    }

    /// Resolve builtin call with class __iter__/__next__ overrides and Map handling.
    /// Returns `None` if the standard `resolve_builtin_call_type` should be used
    /// without class overrides (i.e., no special handling needed).
    fn resolve_builtin_with_overrides(
        &self,
        builtin: &hir::Builtin,
        args: &[hir::ExprId],
        arg_types: &[Type],
        module: &hir::Module,
    ) -> Option<Type> {
        if matches!(builtin, hir::Builtin::Iter) && !arg_types.is_empty() {
            if let Type::Class { class_id, .. } = &arg_types[0] {
                if self
                    .get_class_info(class_id)
                    .and_then(|info| info.get_dunder_func("__iter__"))
                    .is_some()
                {
                    return Some(arg_types[0].clone());
                }
            }
        }
        if matches!(builtin, hir::Builtin::Next) && !arg_types.is_empty() {
            if let Type::Class { class_id, .. } = &arg_types[0] {
                if let Some(ret) = self
                    .get_class_info(class_id)
                    .and_then(|info| info.get_dunder_func("__next__"))
                    .and_then(|func_id| self.get_func_return_type(&func_id).cloned())
                {
                    return Some(ret);
                }
            }
        }
        if let Some(ty) = helpers::resolve_builtin_call_type(builtin, args, arg_types, module) {
            return Some(ty);
        }
        if matches!(builtin, hir::Builtin::Map) {
            let elem_type = if args.len() >= 2 {
                let func_expr = &module.exprs[args[0]];
                // §P.2.2: also resolve `Var` callable args by looking up the
                // closure / function reference recorded during lowering. Without
                // this, `fn = lambda x: ...; map(fn, ...)` resolves to
                // `Iterator[Any]` (no func_id from `Var`), so the for-loop's
                // `IterAdvance` Protocol arm doesn't emit `UnwrapValueInt` for
                // the lambda's tagged-int return — the raw bits leak into the
                // HeapAny shadow-stack slot and trip the GC alignment guard.
                let func_id = match &func_expr.kind {
                    hir::ExprKind::FuncRef(id) => Some(*id),
                    hir::ExprKind::Closure { func, .. } => Some(*func),
                    hir::ExprKind::Var(var_id) => self
                        .get_var_closure(var_id)
                        .map(|(fid, _)| *fid)
                        .or_else(|| self.get_var_func(var_id)),
                    _ => None,
                };
                if let Some(func_id) = func_id {
                    if let Some(return_type) = self.get_func_return_type(&func_id) {
                        return_type.clone()
                    } else if let Some(func_def) = module.func_defs.get(&func_id) {
                        func_def.return_type.clone().unwrap_or(Type::Any)
                    } else {
                        Type::Any
                    }
                } else {
                    Type::Any
                }
            } else {
                Type::Any
            };
            return Some(Type::Iterator(Box::new(elem_type)));
        }
        None
    }

    /// Resolve attribute type on an already-resolved object type.
    /// Returns `None` if no resolution found (caller applies fallback).
    fn resolve_attribute_on_type(&self, obj_ty: &Type, attr: InternedString) -> Option<Type> {
        if let Type::RuntimeObject(type_tag) = obj_ty {
            let attr_name = self.resolve(attr);
            if let Some(field_def) = lookup_object_field(*type_tag, attr_name) {
                return Some(typespec_to_type(&field_def.field_type));
            }
            return Some(Type::Any);
        }
        if matches!(obj_ty, Type::File(_)) {
            let attr_name = self.resolve(attr);
            return Some(match attr_name {
                "closed" => Type::Bool,
                "name" => Type::Str,
                _ => Type::Any,
            });
        }
        // `type(x).__name__` — `type(x)` already returns a `Str` (the
        // `<class 'X'>` form), and the lowering (attributes.rs) turns
        // `__name__` on a str into a call to `rt_type_name_extract`, which
        // returns the bare class name as a `Str`. Type-plan it as `Str` so
        // `print`/`==` go through the string-dispatch paths instead of the
        // raw-i64 `Any` fallback.
        if matches!(obj_ty, Type::Str) && self.resolve(attr) == "__name__" {
            return Some(Type::Str);
        }
        // Handle built-in exception attributes (.args, __class__)
        if matches!(obj_ty, Type::BuiltinException(_)) {
            let attr_name = self.resolve(attr);
            return Some(match attr_name {
                "args" => Type::tuple_of(vec![Type::Str]),
                "__class__" => Type::Str,
                _ => Type::Any,
            });
        }
        if let Type::Class { class_id, .. } = obj_ty {
            if let Some(class_info) = self.get_class_info(class_id) {
                // Handle __class__ on exception class instances
                let attr_name = self.resolve(attr);
                if attr_name == "__class__" && class_info.is_exception_class {
                    return Some(Type::Str);
                }
                if let Some(field_ty) = self.get_refined_class_field_type(class_id, &attr) {
                    return Some(field_ty.clone());
                }
                if let Some(field_ty) = class_info.field_types.get(&attr) {
                    return Some(field_ty.clone());
                }
                if let Some(prop_ty) = class_info.property_types.get(&attr) {
                    return Some(prop_ty.clone());
                }
                if let Some(attr_ty) = class_info.class_attr_types.get(&attr) {
                    return Some(attr_ty.clone());
                }
            }
            // Fall through to cross-module class info (for classes imported
            // from other modules — no local `class_info` entry exists).
            if let Some(info) = self.get_cross_module_class_info(class_id) {
                if let Some(field_ty) = info.field_types.get(&attr) {
                    return Some(field_ty.clone());
                }
            }
        }
        None
    }

    /// Resolve the return type of a `GeneratorIntrinsic` expression.
    ///
    /// `expr_ty` — the HIR-annotated type on the expression node (`expr.ty`),
    ///   used as a fallback for `GetLocal` and `IterNextNoExc`.
    /// `iter_ty` — the already-resolved type of the iterator operand for
    ///   `IterNextNoExc`; callers resolve it via their own sub-expression
    ///   strategy before calling this helper (memoized vs. direct recursion).
    ///
    /// Both `compute_expr_type` and `infer_expr_type_inner` delegate here
    /// after resolving `iter_ty` with their respective strategies.
    fn resolve_generator_intrinsic_type(
        &self,
        intrinsic: &hir::GeneratorIntrinsic,
        expr_ty: Option<&Type>,
        iter_ty: Type,
    ) -> Type {
        match intrinsic {
            hir::GeneratorIntrinsic::Create { .. } => expr_ty
                .cloned()
                .unwrap_or_else(|| Type::Iterator(Box::new(Type::Any))),
            hir::GeneratorIntrinsic::IsExhausted(_)
            | hir::GeneratorIntrinsic::IterIsExhausted(_) => Type::Bool,
            // GetLocal: slot storage is type-erased (i64); desugaring annotates
            // `expr.ty` with the logical Python type stored in the slot.
            hir::GeneratorIntrinsic::GetLocal { .. } => expr_ty.cloned().unwrap_or(Type::Int),
            // IterNextNoExc: element type flows from the iterator being advanced.
            // Prefer the real iterator's element type (via `get_iterable_info`)
            // so tuple iterators like `zip(a, b)` propagate `Tuple<..>` downstream
            // even when desugaring couldn't compute it from the raw AST.
            hir::GeneratorIntrinsic::IterNextNoExc(_) => {
                if let Some((_k, elem)) = crate::utils::get_iterable_info(&iter_ty) {
                    elem
                } else {
                    expr_ty.cloned().unwrap_or(Type::Int)
                }
            }
            _ => Type::Int,
        }
    }

    /// Resolve index type with __getitem__ fallback for classes.
    fn resolve_index_with_getitem(&self, obj_ty: &Type, index_expr: &hir::Expr) -> Option<Type> {
        let base = helpers::resolve_index_type(obj_ty, index_expr);
        if base != Type::Any {
            return Some(base);
        }
        if let Type::Class { class_id, .. } = obj_ty {
            if let Some(ty) = self
                .get_class_info(class_id)
                .and_then(|info| info.get_dunder_func("__getitem__"))
                .and_then(|func_id| self.get_func_return_type(&func_id).cloned())
            {
                return Some(ty);
            }
        }
        None
    }
}

// =============================================================================
// Codegen path: memoized sub-expression resolution via seed_expr_type_by_id
// =============================================================================

impl<'a> Lowering<'a> {
    /// **Internal** — codegen-path implementation of the memoized HIR type
    /// query. External callers must go through `seed_expr_type_by_id`
    /// (which wraps this with caching). S1.9b merges this with
    /// [`Self::infer_seed_expr_type_inner`] into a single unified match.
    ///
    /// Uses `seed_expr_type_by_id` for sub-expressions to ensure caching.
    pub(super) fn compute_seed_expr_type(
        &mut self,
        expr: &hir::Expr,
        hir_module: &hir::Module,
    ) -> Type {
        match &expr.kind {
            hir::ExprKind::Var(var_id) => self
                .get_base_var_type(var_id)
                .cloned()
                .or_else(|| expr.ty.clone())
                .unwrap_or(Type::Any),
            hir::ExprKind::BinOp { op, left, right } => {
                let left_ty = self.seed_expr_type_by_id(*left, hir_module);
                let right_ty = self.seed_expr_type_by_id(*right, hir_module);
                self.binop_result_type(op, &left_ty, &right_ty, expr)
            }
            hir::ExprKind::UnOp { op, operand } => match op {
                hir::UnOp::Not => Type::Bool,
                hir::UnOp::Neg | hir::UnOp::Pos => self.seed_expr_type_by_id(*operand, hir_module),
                hir::UnOp::Invert => Type::Int,
            },
            hir::ExprKind::Compare { .. } => Type::Bool,
            hir::ExprKind::LogicalOp { left, right, .. } => {
                let left_ty = self.seed_expr_type_by_id(*left, hir_module);
                let right_ty = self.seed_expr_type_by_id(*right, hir_module);
                self.logical_op_result_type(left_ty, right_ty)
            }
            hir::ExprKind::IfExpr {
                cond,
                then_val,
                else_val,
            } => {
                // Apply isinstance narrowing to the ternary branches so
                // `x if isinstance(x, T) else T(x)` infers as `T` at the
                // lowering path (§G.13). Without this, the `other = other
                // if isinstance(other, Value) else Value(other)` pattern
                // used in Value.__add__ and friends keeps `other` widened
                // to `Any`, and subsequent `other.data` fails with
                // "unknown attribute 'data'".
                let cond_expr = &hir_module.exprs[*cond];
                let narrow = self.extract_simple_isinstance_narrowing(cond_expr, hir_module, None);
                match narrow {
                    Some((var_id, then_narrow, else_narrow)) => {
                        let then_expr = &hir_module.exprs[*then_val];
                        let else_expr = &hir_module.exprs[*else_val];
                        let then_ty = if matches!(
                            &then_expr.kind,
                            hir::ExprKind::Var(v) if *v == var_id
                        ) {
                            then_narrow
                        } else {
                            self.seed_expr_type_by_id(*then_val, hir_module)
                        };
                        let else_ty = if matches!(
                            &else_expr.kind,
                            hir::ExprKind::Var(v) if *v == var_id
                        ) {
                            else_narrow
                        } else {
                            self.seed_expr_type_by_id(*else_val, hir_module)
                        };
                        helpers::union_or_any(then_ty, else_ty)
                    }
                    None => {
                        let then_ty = self.seed_expr_type_by_id(*then_val, hir_module);
                        let else_ty = self.seed_expr_type_by_id(*else_val, hir_module);
                        helpers::union_or_any(then_ty, else_ty)
                    }
                }
            }
            hir::ExprKind::List(elements) => {
                let elem_types: Vec<Type> = elements
                    .iter()
                    .map(|e| self.seed_expr_type_by_id(*e, hir_module))
                    .collect();
                helpers::infer_list_type(elem_types, expr.ty.as_ref())
            }
            hir::ExprKind::Tuple(elements) => {
                let elem_types: Vec<Type> = elements
                    .iter()
                    .map(|e| self.seed_expr_type_by_id(*e, hir_module))
                    .collect();
                Type::tuple_of(elem_types)
            }
            hir::ExprKind::Dict(pairs) => {
                let key_types: Vec<Type> = pairs
                    .iter()
                    .map(|(k, _)| self.seed_expr_type_by_id(*k, hir_module))
                    .collect();
                let val_types: Vec<Type> = pairs
                    .iter()
                    .map(|(_, v)| self.seed_expr_type_by_id(*v, hir_module))
                    .collect();
                helpers::infer_dict_type(key_types, val_types)
            }
            hir::ExprKind::Set(elements) => {
                let elem_types: Vec<Type> = elements
                    .iter()
                    .map(|e| self.seed_expr_type_by_id(*e, hir_module))
                    .collect();
                helpers::infer_set_type(elem_types)
            }
            hir::ExprKind::MethodCall { obj, method, .. } => {
                let obj_ty = self.seed_expr_type_by_id(*obj, hir_module);
                self.method_call_result_type(&obj_ty, *method, hir_module, expr)
            }
            hir::ExprKind::Slice { obj, .. } => self.seed_expr_type_by_id(*obj, hir_module),
            hir::ExprKind::Index { obj, index } => {
                let obj_ty = self.seed_expr_type_by_id(*obj, hir_module);
                let index_expr = &hir_module.exprs[*index];
                self.index_result_type(&obj_ty, index_expr, expr)
            }
            hir::ExprKind::Call { func, .. } => {
                let func_expr = &hir_module.exprs[*func];
                self.call_result_type(func_expr, hir_module, expr)
            }
            hir::ExprKind::BuiltinCall { builtin, args, .. } => {
                let arg_types: Vec<Type> = args
                    .iter()
                    .map(|id| self.seed_expr_type_by_id(*id, hir_module))
                    .collect();
                self.builtin_call_result_type(builtin, args, &arg_types, hir_module, expr)
            }
            hir::ExprKind::StdlibCall { func, args } => {
                let declared = typespec_to_type(&func.return_type);
                if !matches!(declared, Type::Any | Type::HeapAny) {
                    declared
                } else if let Some(annotated) = expr.ty.clone() {
                    if !matches!(annotated, Type::Any | Type::HeapAny) {
                        annotated
                    } else {
                        match func.name {
                            "choice" => args
                                .first()
                                .map(|arg_id| {
                                    extract_iterable_first_element_type(
                                        &self.seed_expr_type_by_id(*arg_id, hir_module),
                                    )
                                })
                                .unwrap_or(Type::Any),
                            "sample" | "choices" => args
                                .first()
                                .map(|arg_id| {
                                    Type::list_of(extract_iterable_first_element_type(
                                        &self.seed_expr_type_by_id(*arg_id, hir_module),
                                    ))
                                })
                                .unwrap_or(Type::Any),
                            _ => annotated,
                        }
                    }
                } else {
                    declared
                }
            }
            hir::ExprKind::StdlibAttr(attr_def) => typespec_to_type(&attr_def.ty),
            hir::ExprKind::StdlibConst(const_def) => typespec_to_type(&const_def.ty),
            hir::ExprKind::Attribute { obj, attr } => {
                let obj_ty = self.seed_expr_type_by_id(*obj, hir_module);
                self.attribute_result_type(&obj_ty, *attr, expr)
            }
            hir::ExprKind::ClassRef(class_id) => self.class_ref_type(*class_id, hir_module),
            hir::ExprKind::ClassAttrRef { class_id, attr } => {
                self.class_attr_ref_type(*class_id, *attr)
            }
            hir::ExprKind::Closure { func, .. } => self.closure_result_type(*func, hir_module),
            hir::ExprKind::ModuleAttr { module, attr } => {
                let attr_name = self.resolve(*attr).to_string();
                self.module_export_type(module, &attr_name)
            }
            hir::ExprKind::ImportedRef { module, name } => self.module_export_type(module, name),
            // Generator intrinsics — delegate to shared helper; resolve
            // iter_ty here via the memoized seed_expr_type_by_id path.
            hir::ExprKind::GeneratorIntrinsic(intrinsic) => {
                let iter_ty = if let hir::GeneratorIntrinsic::IterNextNoExc(iter_id) = intrinsic {
                    self.seed_expr_type_by_id(*iter_id, hir_module)
                } else {
                    Type::Any
                };
                self.resolve_generator_intrinsic_type(intrinsic, expr.ty.as_ref(), iter_ty)
            }
            _ => expr.ty.clone().unwrap_or(Type::Any),
        }
    }
}

// =============================================================================
// Pre-scan path: direct recursion without memoization
// =============================================================================

impl<'a> Lowering<'a> {
    /// **Public** — pre-scan entry point with a parameter-type overlay.
    /// Use this from any `type_planning/*` walker that has pre-computed
    /// types for unassigned parameters (e.g. lambda/closure capture
    /// analysis, container refinement). Non-memoized — caller pays the
    /// full sub-expression walk on every call. Pass `&IndexMap::new()`
    /// when no overlay is needed (the former `infer_expr_type`
    /// no-overlay wrapper was deleted in §1.4u step 1 since its sole
    /// caller migrated).
    pub(crate) fn seed_infer_expr_type(
        &self,
        expr: &hir::Expr,
        module: &hir::Module,
        param_types: &IndexMap<VarId, Type>,
    ) -> Type {
        self.infer_seed_expr_type_inner(expr, module, Some(param_types))
    }

    /// **Internal** — pre-scan inference engine. Direct recursion, no
    /// memoization. External callers must go through
    /// [`Self::seed_infer_expr_type`]. Same match arms as
    /// [`Self::compute_seed_expr_type`] but different sub-expression
    /// resolution and variable lookup strategy — full unification
    /// (§1.4u) requires the borrow-checker work documented at the top
    /// of this file.
    pub(super) fn infer_seed_expr_type_inner(
        &self,
        expr: &hir::Expr,
        module: &hir::Module,
        param_types: Option<&IndexMap<VarId, Type>>,
    ) -> Type {
        match &expr.kind {
            // === Literals ===
            hir::ExprKind::Int(_) => Type::Int,
            hir::ExprKind::Float(_) => Type::Float,
            hir::ExprKind::Bool(_) => Type::Bool,
            hir::ExprKind::Str(_) => Type::Str,
            hir::ExprKind::Bytes(_) => Type::Bytes,
            hir::ExprKind::None => Type::None,
            hir::ExprKind::NotImplemented => Type::NotImplementedT,

            hir::ExprKind::Var(var_id) => {
                if let Some(pt) = param_types {
                    pt.get(var_id)
                        .cloned()
                        .or_else(|| self.get_var_type(var_id).cloned())
                        .unwrap_or(Type::Any)
                } else {
                    self.get_var_type(var_id)
                        .cloned()
                        .or_else(|| expr.ty.clone())
                        .unwrap_or(Type::Any)
                }
            }
            hir::ExprKind::BinOp { op, left, right } => {
                let left_ty =
                    self.infer_seed_expr_type_inner(&module.exprs[*left], module, param_types);
                let right_ty =
                    self.infer_seed_expr_type_inner(&module.exprs[*right], module, param_types);
                self.binop_result_type(op, &left_ty, &right_ty, expr)
            }
            hir::ExprKind::UnOp { op, operand } => match op {
                hir::UnOp::Not => Type::Bool,
                hir::UnOp::Neg | hir::UnOp::Pos => {
                    self.infer_seed_expr_type_inner(&module.exprs[*operand], module, param_types)
                }
                hir::UnOp::Invert => Type::Int,
            },
            hir::ExprKind::Compare { .. } => Type::Bool,
            hir::ExprKind::LogicalOp { left, right, .. } => {
                let left_ty =
                    self.infer_seed_expr_type_inner(&module.exprs[*left], module, param_types);
                let right_ty =
                    self.infer_seed_expr_type_inner(&module.exprs[*right], module, param_types);
                self.logical_op_result_type(left_ty, right_ty)
            }
            hir::ExprKind::IfExpr {
                cond,
                then_val,
                else_val,
            } => {
                // Apply isinstance narrowing to the ternary branches so
                // `x if isinstance(x, T) else T(x)` infers `T` instead of
                // `Union[Any, T]` (§G.13). Without this, unannotated-param
                // idioms like
                //     other = other if isinstance(other, Value) else Value(other)
                // keep `other` as `Any` and subsequent `other.data` fails
                // with "unknown attribute".
                let cond_expr = &module.exprs[*cond];
                let narrow =
                    self.extract_simple_isinstance_narrowing(cond_expr, module, param_types);
                match narrow {
                    Some((var_id, then_narrow, else_narrow)) => {
                        let mut then_overlay = param_types.cloned().unwrap_or_else(IndexMap::new);
                        then_overlay.insert(var_id, then_narrow);
                        let mut else_overlay = param_types.cloned().unwrap_or_else(IndexMap::new);
                        else_overlay.insert(var_id, else_narrow);
                        let then_ty = self.infer_seed_expr_type_inner(
                            &module.exprs[*then_val],
                            module,
                            Some(&then_overlay),
                        );
                        let else_ty = self.infer_seed_expr_type_inner(
                            &module.exprs[*else_val],
                            module,
                            Some(&else_overlay),
                        );
                        helpers::union_or_any(then_ty, else_ty)
                    }
                    None => {
                        let then_ty = self.infer_seed_expr_type_inner(
                            &module.exprs[*then_val],
                            module,
                            param_types,
                        );
                        let else_ty = self.infer_seed_expr_type_inner(
                            &module.exprs[*else_val],
                            module,
                            param_types,
                        );
                        helpers::union_or_any(then_ty, else_ty)
                    }
                }
            }
            hir::ExprKind::List(elements) => {
                let elem_types: Vec<Type> = elements
                    .iter()
                    .map(|e| {
                        self.infer_seed_expr_type_inner(&module.exprs[*e], module, param_types)
                    })
                    .collect();
                helpers::infer_list_type(elem_types, expr.ty.as_ref())
            }
            hir::ExprKind::Tuple(elements) => {
                let elem_types: Vec<Type> = elements
                    .iter()
                    .map(|e| {
                        self.infer_seed_expr_type_inner(&module.exprs[*e], module, param_types)
                    })
                    .collect();
                Type::tuple_of(elem_types)
            }
            hir::ExprKind::Dict(pairs) => {
                let key_types: Vec<Type> = pairs
                    .iter()
                    .map(|(k, _)| {
                        self.infer_seed_expr_type_inner(&module.exprs[*k], module, param_types)
                    })
                    .collect();
                let val_types: Vec<Type> = pairs
                    .iter()
                    .map(|(_, v)| {
                        self.infer_seed_expr_type_inner(&module.exprs[*v], module, param_types)
                    })
                    .collect();
                helpers::infer_dict_type(key_types, val_types)
            }
            hir::ExprKind::Set(elements) => {
                let elem_types: Vec<Type> = elements
                    .iter()
                    .map(|e| {
                        self.infer_seed_expr_type_inner(&module.exprs[*e], module, param_types)
                    })
                    .collect();
                helpers::infer_set_type(elem_types)
            }
            hir::ExprKind::MethodCall { obj, method, .. } => {
                let obj_ty =
                    self.infer_seed_expr_type_inner(&module.exprs[*obj], module, param_types);
                self.method_call_result_type(&obj_ty, *method, module, expr)
            }
            hir::ExprKind::Slice { obj, .. } => {
                self.infer_seed_expr_type_inner(&module.exprs[*obj], module, param_types)
            }
            hir::ExprKind::Index { obj, index } => {
                let obj_ty =
                    self.infer_seed_expr_type_inner(&module.exprs[*obj], module, param_types);
                let index_expr = &module.exprs[*index];
                self.index_result_type(&obj_ty, index_expr, expr)
            }
            hir::ExprKind::Call { func, .. } => {
                let func_expr = &module.exprs[*func];
                self.call_result_type(func_expr, module, expr)
            }
            hir::ExprKind::BuiltinCall { builtin, args, .. } => {
                let arg_types: Vec<Type> = args
                    .iter()
                    .map(|id| {
                        self.infer_seed_expr_type_inner(&module.exprs[*id], module, param_types)
                    })
                    .collect();
                self.builtin_call_result_type(builtin, args, &arg_types, module, expr)
            }
            hir::ExprKind::StdlibCall { func, args } => {
                let declared = typespec_to_type(&func.return_type);
                if !matches!(declared, Type::Any | Type::HeapAny) {
                    declared
                } else if let Some(annotated) = expr.ty.clone() {
                    if !matches!(annotated, Type::Any | Type::HeapAny) {
                        annotated
                    } else {
                        match func.name {
                            "choice" => args
                                .first()
                                .map(|arg_id| {
                                    extract_iterable_first_element_type(
                                        &self.infer_seed_expr_type_inner(
                                            &module.exprs[*arg_id],
                                            module,
                                            param_types,
                                        ),
                                    )
                                })
                                .unwrap_or(Type::Any),
                            "sample" | "choices" => args
                                .first()
                                .map(|arg_id| {
                                    Type::list_of(extract_iterable_first_element_type(
                                        &self.infer_seed_expr_type_inner(
                                            &module.exprs[*arg_id],
                                            module,
                                            param_types,
                                        ),
                                    ))
                                })
                                .unwrap_or(Type::Any),
                            _ => annotated,
                        }
                    }
                } else {
                    declared
                }
            }
            hir::ExprKind::StdlibAttr(attr_def) => typespec_to_type(&attr_def.ty),
            hir::ExprKind::StdlibConst(const_def) => typespec_to_type(&const_def.ty),
            hir::ExprKind::Attribute { obj, attr } => {
                let obj_ty =
                    self.infer_seed_expr_type_inner(&module.exprs[*obj], module, param_types);
                self.attribute_result_type(&obj_ty, *attr, expr)
            }
            hir::ExprKind::ClassRef(class_id) => self.class_ref_type(*class_id, module),
            hir::ExprKind::ClassAttrRef { class_id, attr } => {
                self.class_attr_ref_type(*class_id, *attr)
            }
            hir::ExprKind::Closure { func, .. } => self.closure_result_type(*func, module),
            hir::ExprKind::ModuleAttr {
                module: mod_name,
                attr,
            } => {
                let attr_name = self.resolve(*attr).to_string();
                self.module_export_type(mod_name, &attr_name)
            }
            hir::ExprKind::ImportedRef {
                module: mod_name,
                name,
            } => self.module_export_type(mod_name, name),
            // Generator intrinsics — delegate to shared helper; resolve
            // iter_ty here via the direct (non-memoized) recursion path.
            hir::ExprKind::GeneratorIntrinsic(intrinsic) => {
                let iter_ty = if let hir::GeneratorIntrinsic::IterNextNoExc(iter_id) = intrinsic {
                    self.infer_seed_expr_type_inner(&module.exprs[*iter_id], module, param_types)
                } else {
                    Type::Any
                };
                self.resolve_generator_intrinsic_type(intrinsic, expr.ty.as_ref(), iter_ty)
            }
            _ => expr.ty.clone().unwrap_or(Type::Any),
        }
    }

    /// Extract `isinstance(var, T)` (or `not isinstance(var, T)`) narrowing
    /// info from a condition expression. Returns `(var_id, then_ty, else_ty)`
    /// with the narrowing applied. Used by the `IfExpr` arm of
    /// `infer_seed_expr_type_inner` to give ternary branches refined types.
    ///
    /// Unlike `narrowing::extract_isinstance_info`, this helper consults the
    /// supplied `param_types` overlay first so it works during pre-scan
    /// (before `self.symbols.var_types` is populated for the function
    /// currently being analysed).
    fn extract_simple_isinstance_narrowing(
        &self,
        cond: &hir::Expr,
        module: &hir::Module,
        param_types: Option<&IndexMap<VarId, Type>>,
    ) -> Option<(VarId, Type, Type)> {
        let (args, negated) = match &cond.kind {
            hir::ExprKind::BuiltinCall {
                builtin: hir::Builtin::Isinstance,
                args,
                ..
            } => (args.as_slice(), false),
            hir::ExprKind::UnOp {
                op: hir::UnOp::Not,
                operand,
            } => {
                let inner = &module.exprs[*operand];
                if let hir::ExprKind::BuiltinCall {
                    builtin: hir::Builtin::Isinstance,
                    args,
                    ..
                } = &inner.kind
                {
                    (args.as_slice(), true)
                } else {
                    return None;
                }
            }
            _ => return None,
        };

        if args.len() < 2 {
            return None;
        }
        let obj_expr = &module.exprs[args[0]];
        let type_expr = &module.exprs[args[1]];

        let var_id = match &obj_expr.kind {
            hir::ExprKind::Var(v) => *v,
            _ => return None,
        };
        let checked_type = match &type_expr.kind {
            hir::ExprKind::TypeRef(ty) => ty.clone(),
            hir::ExprKind::ClassRef(class_id) => {
                let class_def = module.class_defs.get(class_id)?;
                Type::Class {
                    class_id: *class_id,
                    name: class_def.name,
                }
            }
            _ => return None,
        };

        // Resolve the var's base type: overlay first, then the lowering's
        // stable state (refined/prescan/global), else default to Any.
        // §1.4u-b: this helper is called from `compute_expr_type`'s
        // IfExpr arm, which must be free of `symbols.var_types` reads so
        // non-Var expressions can be eagerly cached in `build_lowering_seed_info`.
        // The narrowing heuristic works on the base (pre-narrowing) type
        // anyway — narrowing INSIDE an isinstance condition is the
        // outer rule the IfExpr is applying; reading the post-narrowing
        // value here would just restate the rule.
        let original_type = param_types
            .and_then(|pt| pt.get(&var_id).cloned())
            .or_else(|| self.get_base_var_type(&var_id).cloned())
            .unwrap_or(Type::Any);

        let narrowed = original_type.meet(&checked_type);
        let excluded = original_type.minus(&checked_type);
        if negated {
            Some((var_id, excluded, narrowed))
        } else {
            Some((var_id, narrowed, excluded))
        }
    }
}

/// Extract the element type from an iterable type.
pub(crate) fn extract_iterable_element_type(ty: &Type) -> Type {
    if let Some(elem) = ty.list_elem() {
        return elem.clone();
    }
    if let Some(elems) = ty.tuple_elems() {
        return if !elems.is_empty() {
            elems
                .iter()
                .cloned()
                .reduce(|a, b| a.join(&b))
                .unwrap_or(Type::Never)
        } else {
            Type::Any
        };
    }
    if let Some(elem) = ty.tuple_var_elem() {
        return elem.clone();
    }
    if let Some(elem) = ty.set_elem() {
        return elem.clone();
    }
    if let Some((key, _)) = ty.dict_kv() {
        return key.clone();
    }
    match ty {
        Type::Str => Type::Str,
        Type::Bytes => Type::Int,
        Type::Iterator(elem) => (**elem).clone(),
        _ => Type::Any,
    }
}

/// Extract element type for iteration contexts.
/// Unlike `extract_iterable_element_type` which computes the union of all tuple elements,
/// this returns only the first tuple element type — appropriate for iteration over
/// homogeneous containers where the first element represents the common type.
pub(crate) fn extract_iterable_first_element_type(ty: &Type) -> Type {
    if let Some(elem) = ty.list_elem() {
        return elem.clone();
    }
    if let Some(elems) = ty.tuple_elems() {
        return if !elems.is_empty() {
            elems[0].clone()
        } else {
            Type::Any
        };
    }
    if let Some(elem) = ty.tuple_var_elem() {
        return elem.clone();
    }
    if let Some(elem) = ty.set_elem() {
        return elem.clone();
    }
    if let Some((key, _)) = ty.dict_kv() {
        return key.clone();
    }
    match ty {
        Type::Str => Type::Str,
        Type::Bytes => Type::Int,
        Type::Iterator(elem) => (**elem).clone(),
        _ => Type::Any,
    }
}

// =============================================================================
// S1.9b shared result-computation helpers
// =============================================================================
//
// Each helper takes **already-resolved sub-expression types** and produces the
// parent expression's result type. The helper body is identical to what used
// to live inline in both `compute_expr_type` and `infer_expr_type_inner`,
// factored out so the two dispatchers share logic without needing to share
// sub-expression-recursion strategy.
//
// `&self` — safe to call from both the memoized (`&mut self`) and pre-scan
// (`&self`) paths.

impl<'a> Lowering<'a> {
    /// Result type of `left op right`. Prefers a class-dunder's inferred
    /// return type, falls back to the numeric-tower helper, then to the
    /// HIR-annotated `expr.ty`, then `Any`.
    pub(super) fn binop_result_type(
        &self,
        op: &hir::BinOp,
        left_ty: &Type,
        right_ty: &Type,
        expr: &hir::Expr,
    ) -> Type {
        if let Some(ty) = self.resolve_class_binop_return(op, left_ty, right_ty) {
            ty
        } else {
            helpers::resolve_binop_type(op, left_ty, right_ty)
                .unwrap_or_else(|| expr.ty.clone().unwrap_or(Type::Any))
        }
    }

    /// Result type of `left and right` / `left or right`. Python's
    /// short-circuit ops return one of the operands, not a Bool — the
    /// conservative upper bound is their union.
    pub(super) fn logical_op_result_type(&self, left_ty: Type, right_ty: Type) -> Type {
        helpers::union_or_any(left_ty, right_ty)
    }

    /// Result type of `obj.method(...)`. Dispatches via
    /// `resolve_method_on_type`, falling back to `expr.ty` / `Any`.
    pub(super) fn method_call_result_type(
        &self,
        obj_ty: &Type,
        method: pyaot_utils::InternedString,
        module: &hir::Module,
        expr: &hir::Expr,
    ) -> Type {
        let unwrapped = helpers::unwrap_optional(obj_ty);
        let method_name = self.resolve(method);
        self.resolve_method_on_type(&unwrapped, method, method_name, module)
            .unwrap_or_else(|| expr.ty.clone().unwrap_or(Type::Any))
    }

    /// Result type of `obj[index]` via the `__getitem__` resolution
    /// helper. Falls back to `expr.ty` / `Any`.
    pub(super) fn index_result_type(
        &self,
        obj_ty: &Type,
        index_expr: &hir::Expr,
        expr: &hir::Expr,
    ) -> Type {
        self.resolve_index_with_getitem(obj_ty, index_expr)
            .unwrap_or_else(|| expr.ty.clone().unwrap_or(Type::Any))
    }

    /// Result type of a regular function `Call`. Delegates to
    /// `resolve_call_target_type`; falls back to `expr.ty` / `Any`.
    pub(super) fn call_result_type(
        &self,
        func_expr: &hir::Expr,
        module: &hir::Module,
        expr: &hir::Expr,
    ) -> Type {
        self.resolve_call_target_type(func_expr, module)
            .unwrap_or_else(|| expr.ty.clone().unwrap_or(Type::Any))
    }

    /// Result type of a `BuiltinCall`. Delegates to
    /// `resolve_builtin_with_overrides`; falls back to `expr.ty` / `Any`.
    pub(crate) fn builtin_call_result_type(
        &self,
        builtin: &hir::Builtin,
        args: &[hir::ExprId],
        arg_types: &[Type],
        module: &hir::Module,
        expr: &hir::Expr,
    ) -> Type {
        self.resolve_builtin_with_overrides(builtin, args, arg_types, module)
            .unwrap_or_else(|| expr.ty.clone().unwrap_or(Type::Any))
    }

    /// Result type of `obj.attr`. Delegates to `resolve_attribute_on_type`;
    /// falls back to `expr.ty` / `Any`.
    pub(crate) fn attribute_result_type(
        &self,
        obj_ty: &Type,
        attr: pyaot_utils::InternedString,
        expr: &hir::Expr,
    ) -> Type {
        self.resolve_attribute_on_type(obj_ty, attr)
            .unwrap_or_else(|| expr.ty.clone().unwrap_or(Type::Any))
    }

    /// `ClassRef(id)` → `Type::Class { id, name }` if the class is
    /// known in the module; else `Any`.
    pub(super) fn class_ref_type(
        &self,
        class_id: pyaot_utils::ClassId,
        module: &hir::Module,
    ) -> Type {
        match module.class_defs.get(&class_id) {
            Some(class_def) => Type::Class {
                class_id,
                name: class_def.name,
            },
            None => Type::Any,
        }
    }

    /// `ClassAttrRef { class_id, attr }` — lookup in the class's
    /// attribute-type table; falls back to `Any`.
    pub(super) fn class_attr_ref_type(
        &self,
        class_id: pyaot_utils::ClassId,
        attr: pyaot_utils::InternedString,
    ) -> Type {
        self.get_class_info(&class_id)
            .and_then(|info| info.class_attr_types.get(&attr).cloned())
            .unwrap_or(Type::Any)
    }

    /// `Closure { func, … }` → the callee function's declared return
    /// type, or `Any` if unknown.
    pub(super) fn closure_result_type(
        &self,
        func_id: pyaot_utils::FuncId,
        module: &hir::Module,
    ) -> Type {
        module
            .func_defs
            .get(&func_id)
            .and_then(|f| f.return_type.clone())
            .unwrap_or(Type::Any)
    }

    /// `ModuleAttr` / `ImportedRef` — look up an exported variable's type
    /// by (module_path, attribute_name) in the cross-module export
    /// table. Falls back to `Any` if unresolved.
    pub(super) fn module_export_type(&self, module_path: &str, attr: &str) -> Type {
        let key = (module_path.to_string(), attr.to_string());
        self.get_module_var_export(&key)
            .map(|(_var, ty)| ty.clone())
            .unwrap_or(Type::Any)
    }
}
