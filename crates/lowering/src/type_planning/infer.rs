//! Infer mode: bottom-up type synthesis
//!
//! # Unified arm dispatch (single source of truth)
//!
//! [`Lowering::arm_dispatch`] is THE single `match &expr.kind` over every
//! `ExprKind`. The three historical expression-type recursion shells all
//! delegate to it, so a new `ExprKind` (or a changed arm) is edited in
//! ONE place rather than kept in sync across three matches:
//!
//! | Shell | Entry point | Driver |
//! |-------|-------------|--------|
//! | **Planning** | [`Lowering::seed_expr_type_by_id`] | memoized (`&mut self`), reads the eager-populated cache |
//! | **Prescan** | [`Lowering::seed_infer_expr_type`] | direct (`&self`), parameter-type overlay |
//! | **Lowering** | [`Lowering::seed_expr_type`] | flow-set re-eval + cache, `Never`→`Any` coercion |
//!
//! Each shell is a thin wrapper. Per-shell mechanics (caching, coercion,
//! the variable-source chain) live in [`Lowering::seed_sub`] — the ONE
//! place modes differ in how they recurse. `arm_dispatch` itself is mode-
//! agnostic except for a handful of arms gated on [`SeedMode`]:
//! `Var` ([`Lowering::seed_var`]), the literals (Planning defers to
//! `expr.ty`; the others use concrete literal types), `TypeRef`,
//! `NotImplemented`, `FormatSpec`, `IfExpr`, and the catch-all
//! ([`Lowering::seed_catchall`]). Every other arm is identical across all
//! three modes and recurses through `seed_sub(mode)`.
//!
//! ## Why the divergence is gated, not merged
//!
//! The three shells deliberately keep distinct caching/coercion mechanics
//! (the "safe" scope — runtime behavior is preserved exactly):
//!
//! - **Planning** recurses via a read-only cache lookup (no writes during
//!   recursion — cache writes are hoisted to the `&mut`
//!   `seed_expr_type_by_id` wrapper, the only `&mut` entry point). The
//!   eager-populate pass (`eagerly_populate_expr_types`) visits every
//!   `ExprId` in arena order (children < parents), so a sub-expression is
//!   already cached by the time its parent is computed → O(n), equivalent
//!   to the historical by-id memoized recursion.
//! - **Prescan** recurses directly with no cache (it runs before the
//!   memoization cache is populated; caching there would freeze stale
//!   types). Literals resolve to concrete types so unannotated literal
//!   shapes type-plan precisely.
//! - **Lowering** re-evaluates only a fixed flow-set of arms against the
//!   current narrowing state and reads the cache for everything else
//!   (preserving the "unannotated `int+float` stays `Any` at lowering
//!   reads" contract); it coerces `Never`→`Any` at every level.
//!
//! Planning/Prescan never coerce — empty-container narrowing
//! (`list[Never]`), `element_is_bottom` defer, and Union refinement all
//! need raw `Never`. Planning literals fall to `expr.ty` (preserving the
//! `ty:None` defaultdict-factory-tag → `Any` behavior the cache depends
//! on).
//!
//! # Adding new match arms
//!
//! Add the arm to [`Lowering::arm_dispatch`] **once**. Gate it on
//! [`SeedMode`] only where the shells genuinely differ; otherwise recurse
//! into sub-expressions via `seed_sub(*child, module, mode)` so the per-
//! mode caching/coercion is applied uniformly. If the arm has complex
//! logic that doesn't depend on how sub-expressions are resolved, extract
//! it into a `*_result_type` / `resolve_*` helper (takes `&self`,
//! receives pre-computed sub-expression types as arguments) so the table
//! stays a flat dispatch.

use indexmap::IndexMap;
use pyaot_hir as hir;
use pyaot_stdlib_defs::{lookup_object_field, lookup_object_type};
use pyaot_types::{typespec_to_type, Type, TypeLattice};
use pyaot_utils::interner::InternedString;
use pyaot_utils::VarId;

use super::helpers;
use crate::context::Lowering;

/// Which expression-type recursion shell is driving [`Lowering::arm_dispatch`].
///
/// The three shells share every uniform arm and differ ONLY in:
/// - variable resolution ([`Lowering::seed_var`]),
/// - literal handling (Planning defers to `expr.ty`; the others use concrete
///   literal types),
/// - sub-expression recursion + caching + coercion ([`Lowering::seed_sub`]),
/// - the catch-all fallback ([`Lowering::seed_catchall`]).
///
/// See the module-level docs for why the divergence is gated, not merged.
#[derive(Clone, Copy)]
pub(crate) enum SeedMode<'p> {
    /// Planning — [`Lowering::seed_expr_type_by_id`]. Reads the
    /// eager-populated cache for sub-expressions, resolves vars from their
    /// base type, never coerces.
    Planning,
    /// Prescan — [`Lowering::seed_infer_expr_type`]. Direct (non-memoized)
    /// recursion with an optional parameter-type overlay; never coerces.
    Prescan(Option<&'p IndexMap<VarId, Type>>),
    /// Lowering — [`Lowering::seed_expr_type`]. Re-evaluates a fixed
    /// flow-set of arms against the current narrowing state and reads the
    /// cache for everything else; coerces `Never`→`Any` at every level.
    Lowering,
}

/// Lowering-shell boundary coercion. Internal join-state
/// (`current_local_seed_types`, `refined_container_types`, `expr.ty` from
/// empty-literal seeding) may carry `Never` for correct `TypeLattice::join`
/// narrowing; consumers of the effective expression type expect a
/// runtime-safe shape so dispatch / unbox decisions match what MIR/codegen
/// sees. Idempotent — applied at every `seed_sub(Lowering)` level.
fn coerce_lowering_seed(ty: Type) -> Type {
    match ty {
        Type::Never => Type::Any,
        other => other.demote_never_params_to_any(),
    }
}

// =============================================================================
// Shared helpers: type resolution after sub-expressions are computed.
// All take `&self` so both `&mut self` (codegen) and `&self` (pre-scan) can use them.
// =============================================================================

impl<'a> Lowering<'a> {
    /// Resolve method call type on an already-resolved object type.
    /// Returns `None` if no resolution found (caller applies fallback).
    pub(crate) fn resolve_method_on_type(
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

    /// Resolve a function's return type by consulting the type-inference cache
    /// first, then falling back to the HIR's declared `return_type` (defaulting
    /// to `Type::None` for functions with no annotation).
    ///
    /// Returns `None` only if `func_id` is unknown in both the cache and the
    /// module's `func_defs` map — typically a cross-module reference that
    /// hasn't been resolved.
    pub(crate) fn resolve_func_return_type(
        &self,
        func_id: &pyaot_utils::FuncId,
        module: &hir::Module,
    ) -> Option<Type> {
        self.get_func_return_type(func_id).cloned().or_else(|| {
            module
                .func_defs
                .get(func_id)
                .map(|f| f.return_type.clone().unwrap_or(Type::None))
        })
    }

    /// Resolve call target type from the function expression.
    /// Returns `None` if no resolution found (caller applies fallback).
    pub(crate) fn resolve_call_target_type(
        &self,
        func_expr: &hir::Expr,
        module: &hir::Module,
    ) -> Option<Type> {
        if let hir::ExprKind::FuncRef(func_id) = &func_expr.kind {
            if let Some(return_type) = self.resolve_func_return_type(func_id, module) {
                return Some(return_type);
            }
        }
        // Immediate Closure call, e.g. `(Closure { __genexp_N, [captures] })()`
        // emitted by gen-expr desugaring. Return the wrapped function's return
        // type so downstream `sum`/`min`/`max` dispatch sees `Iterator(...)`.
        if let hir::ExprKind::Closure { func: func_id, .. } = &func_expr.kind {
            if let Some(return_type) = self.resolve_func_return_type(func_id, module) {
                return Some(return_type);
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
                    if let Some(return_type) = self.resolve_func_return_type(&call_func_id, module)
                    {
                        return Some(return_type);
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
                if let Some(return_type) = self.resolve_func_return_type(&original_func_id, module)
                {
                    return Some(return_type);
                }
            }
            if let Some((_, original_func_id)) = self.get_module_var_wrapper(var_id) {
                if let Some(return_type) = self.resolve_func_return_type(&original_func_id, module)
                {
                    return Some(return_type);
                }
            }
            if let Some(func_id) = self.get_var_func(var_id) {
                if let Some(return_type) = self.resolve_func_return_type(&func_id, module) {
                    return Some(return_type);
                }
            }
            // Identity-decorated module-level functions: `@identity def f(): …`
            // leaves `f` as a Var pointing at the original FuncId (tracked in
            // `module_var_funcs`, populated by `process_module_decorated_functions`).
            // Required so eager-cache of Call-expr return types sees the original
            // function's return type instead of falling through to `Type::Any`.
            if let Some(func_id) = self.get_module_var_func(var_id) {
                if let Some(return_type) = self.resolve_func_return_type(&func_id, module) {
                    return Some(return_type);
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
    pub(crate) fn resolve_builtin_with_overrides(
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
    pub(crate) fn resolve_attribute_on_type(
        &self,
        obj_ty: &Type,
        attr: InternedString,
    ) -> Option<Type> {
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
        // Both plain class instances (`Type::Class`) and generic
        // instances (`Type::Generic { base, args }`) reach the same
        // field tables — `args` only affects type-var substitution at
        // monomorphization time. Without this `Generic` arm, attribute
        // access on `tb: TickBox[int]` falls through to the
        // `expr.ty.unwrap_or(Any)` branch and the seed-type pipeline
        // sees `Any` for `self.n`, which then poisons the cross-
        // instance harvester (a `BinOp self.n + 1` resolves to `Any`,
        // gets misread as a compound RHS, and falsely marks the field
        // as heap-typed).
        if let Some(class_id) = obj_ty.class_id() {
            if let Some(class_info) = self.get_class_info(&class_id) {
                // Handle __class__ on exception class instances
                let attr_name = self.resolve(attr);
                if attr_name == "__class__" && class_info.is_exception_class {
                    return Some(Type::Str);
                }
                // Refined types (the solver's converged `ClassField`
                // values) are folded into `class_info.field_types` post
                // type-planning, so reading `refined_class_field_types`
                // first and `field_types` second is the single source of
                // truth. An `Any`-typed (heap-carrying) write already
                // widens the field via the solver's `FieldWrite` JOIN, so
                // no separate heap-writes precedence check is needed.
                if let Some(field_ty) = self
                    .lowering_seed_info
                    .refined_class_field_types
                    .get(&class_id)
                    .and_then(|fields| fields.get(&attr))
                {
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
            if let Some(info) = self.get_cross_module_class_info(&class_id) {
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
    /// The `GeneratorIntrinsic` arm of `arm_dispatch` delegates here after
    /// resolving `iter_ty` via the mode-appropriate recursion (`seed_sub`).
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
    pub(crate) fn resolve_index_with_getitem(
        &self,
        obj_ty: &Type,
        index_expr: &hir::Expr,
    ) -> Option<Type> {
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
    /// Result type of a unary operation given the operand's type.
    ///
    /// For a class operand defining the relevant unary dunder, the result is
    /// the dunder's *declared return type* (e.g. `__neg__ -> int`,
    /// `__invert__ -> SameClass`) — not the operator's primitive semantics.
    /// This keeps the inferred expression type consistent with what
    /// `lower_unop` writes into the result slot; otherwise a `__neg__ -> int`
    /// result bound to a class-typed (GC-root) variable holds a raw int and
    /// segfaults on the next pointer-shaped use. For the common
    /// same-type-returning dunders (`__neg__ -> Self`, `__invert__ -> int`)
    /// this returns exactly what the primitive heuristic did, so existing
    /// behaviour is unchanged.
    pub(crate) fn unary_op_result_type(&self, op: hir::UnOp, operand_ty: &Type) -> Type {
        match op {
            // `not x` is always bool (dispatches through `__bool__`, negated).
            hir::UnOp::Not => Type::Bool,
            hir::UnOp::Neg | hir::UnOp::Pos | hir::UnOp::Invert => {
                if let Type::Class { class_id, .. } = operand_ty {
                    if let Some(ret) = self
                        .get_class_info(class_id)
                        .and_then(|ci| ci.get_dunder_func(op.dunder_name()))
                        .and_then(|fid| self.get_func_return_type(&fid).cloned())
                    {
                        return ret;
                    }
                }
                match op {
                    // `-bool`/`+bool` yield int; other operands keep their type.
                    hir::UnOp::Neg | hir::UnOp::Pos => {
                        if matches!(operand_ty, Type::Bool) {
                            Type::Int
                        } else {
                            operand_ty.clone()
                        }
                    }
                    // Bitwise NOT on primitives is always int.
                    hir::UnOp::Invert => Type::Int,
                    _ => unreachable!(),
                }
            }
        }
    }

    /// THE single arm table shared by all three expression-type recursion
    /// shells (Planning / Prescan / Lowering). Recurses into
    /// sub-expressions via [`Self::seed_sub`], which applies the per-mode
    /// caching/coercion. Mode-divergent arms (`Var`, literals, `TypeRef`,
    /// `NotImplemented`, `FormatSpec`, `IfExpr`, the catch-all) are gated
    /// on `mode`; every other arm is identical across all three modes.
    ///
    /// `&self` so it is callable from both the memoized (`&mut self`)
    /// Planning wrapper and the `&self` Prescan/Lowering wrappers.
    pub(crate) fn arm_dispatch(
        &self,
        expr: &hir::Expr,
        module: &hir::Module,
        mode: SeedMode<'_>,
    ) -> Type {
        match &expr.kind {
            // ===== mode-divergent arms =====
            hir::ExprKind::Var(var_id) => self.seed_var(var_id, expr, mode),
            // Literals: Planning defers to `expr.ty` (preserves the
            // `ty:None` factory-tag → `Any` behavior the cache depends
            // on); Prescan/Lowering resolve to the concrete literal type.
            hir::ExprKind::Int(_) => match mode {
                SeedMode::Planning => self.seed_catchall(expr),
                _ => Type::Int,
            },
            hir::ExprKind::Float(_) => match mode {
                SeedMode::Planning => self.seed_catchall(expr),
                _ => Type::Float,
            },
            hir::ExprKind::Bool(_) => match mode {
                SeedMode::Planning => self.seed_catchall(expr),
                _ => Type::Bool,
            },
            hir::ExprKind::Str(_) => match mode {
                SeedMode::Planning => self.seed_catchall(expr),
                _ => Type::Str,
            },
            hir::ExprKind::Bytes(_) => match mode {
                SeedMode::Planning => self.seed_catchall(expr),
                _ => Type::Bytes,
            },
            hir::ExprKind::None => match mode {
                SeedMode::Planning => self.seed_catchall(expr),
                _ => Type::None,
            },
            // TypeRef is an explicit Lowering arm only; Planning/Prescan
            // never had one, so they fall to the catch-all (`expr.ty`).
            hir::ExprKind::TypeRef(ty) => match mode {
                SeedMode::Lowering => ty.clone(),
                _ => self.seed_catchall(expr),
            },
            // NotImplemented / FormatSpec are explicit Prescan arms only.
            hir::ExprKind::NotImplemented => match mode {
                SeedMode::Prescan(_) => Type::NotImplementedT,
                _ => self.seed_catchall(expr),
            },
            hir::ExprKind::FormatSpec { .. } => match mode {
                SeedMode::Prescan(_) => Type::Str,
                _ => self.seed_catchall(expr),
            },
            hir::ExprKind::IfExpr {
                cond,
                then_val,
                else_val,
            } => self.seed_if_expr(*cond, *then_val, *else_val, module, mode),
            // ===== uniform arms (identical across modes) =====
            hir::ExprKind::BinOp { op, left, right } => {
                let left_ty = self.seed_sub(*left, module, mode);
                let right_ty = self.seed_sub(*right, module, mode);
                self.binop_result_type(op, &left_ty, &right_ty, expr)
            }
            hir::ExprKind::UnOp { op, operand } => {
                let operand_ty = self.seed_sub(*operand, module, mode);
                self.unary_op_result_type(*op, &operand_ty)
            }
            hir::ExprKind::Compare { .. } => Type::Bool,
            hir::ExprKind::LogicalOp { left, right, .. } => {
                let left_ty = self.seed_sub(*left, module, mode);
                let right_ty = self.seed_sub(*right, module, mode);
                self.logical_op_result_type(left_ty, right_ty)
            }
            hir::ExprKind::List(elements) => {
                let elem_types: Vec<Type> = elements
                    .iter()
                    .map(|e| self.seed_sub(*e, module, mode))
                    .collect();
                helpers::infer_list_type(elem_types, expr.ty.as_ref())
            }
            hir::ExprKind::Tuple(elements) => {
                let elem_types: Vec<Type> = elements
                    .iter()
                    .map(|e| self.seed_sub(*e, module, mode))
                    .collect();
                Type::tuple_of(elem_types)
            }
            hir::ExprKind::Dict(pairs) => {
                let key_types: Vec<Type> = pairs
                    .iter()
                    .map(|(k, _)| self.seed_sub(*k, module, mode))
                    .collect();
                let val_types: Vec<Type> = pairs
                    .iter()
                    .map(|(_, v)| self.seed_sub(*v, module, mode))
                    .collect();
                helpers::infer_dict_type(key_types, val_types)
            }
            hir::ExprKind::Set(elements) => {
                let elem_types: Vec<Type> = elements
                    .iter()
                    .map(|e| self.seed_sub(*e, module, mode))
                    .collect();
                helpers::infer_set_type(elem_types)
            }
            hir::ExprKind::MethodCall { obj, method, .. } => {
                let obj_ty = self.seed_sub(*obj, module, mode);
                self.method_call_result_type(&obj_ty, *method, module, expr)
            }
            hir::ExprKind::Slice { obj, .. } => self.seed_sub(*obj, module, mode),
            hir::ExprKind::Index { obj, index } => {
                let obj_ty = self.seed_sub(*obj, module, mode);
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
                    .map(|id| self.seed_sub(*id, module, mode))
                    .collect();
                self.builtin_call_result_type(builtin, args, &arg_types, module, expr)
            }
            hir::ExprKind::StdlibCall { func, args } => {
                let declared = typespec_to_type(&func.return_type);
                if !matches!(declared, Type::Any) {
                    declared
                } else if let Some(annotated) = expr.ty.clone() {
                    if !matches!(annotated, Type::Any) {
                        annotated
                    } else {
                        match func.name {
                            "choice" => args
                                .first()
                                .map(|arg_id| {
                                    extract_iterable_first_element_type(
                                        &self.seed_sub(*arg_id, module, mode),
                                    )
                                })
                                .unwrap_or(Type::Any),
                            "sample" | "choices" => args
                                .first()
                                .map(|arg_id| {
                                    Type::list_of(extract_iterable_first_element_type(
                                        &self.seed_sub(*arg_id, module, mode),
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
                let obj_ty = self.seed_sub(*obj, module, mode);
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
            // iter_ty here via the mode-appropriate recursion (`seed_sub`).
            hir::ExprKind::GeneratorIntrinsic(intrinsic) => {
                let iter_ty = if let hir::GeneratorIntrinsic::IterNextNoExc(iter_id) = intrinsic {
                    self.seed_sub(*iter_id, module, mode)
                } else {
                    Type::Any
                };
                self.resolve_generator_intrinsic_type(intrinsic, expr.ty.as_ref(), iter_ty)
            }
            _ => self.seed_catchall(expr),
        }
    }

    /// Per-mode variable-type resolution. The three shells read the
    /// variable's type from different sources:
    ///
    /// - **Planning** (matching the `seed_expr_type_by_id` Var fast-path):
    ///   `get_var_type` → `get_base_var_type` → `expr.ty` → `Any`.
    /// - **Prescan**: overlay (when present) → `get_var_type` → `Any`;
    ///   with no overlay, `get_var_type` → `expr.ty` → `Any`.
    /// - **Lowering**: `block_narrowed_locals` (the current narrowing
    ///   frame) → `get_var_type` → `get_base_var_type` → `expr.ty` → `Any`.
    fn seed_var(&self, var_id: &VarId, expr: &hir::Expr, mode: SeedMode<'_>) -> Type {
        match mode {
            SeedMode::Planning => self
                .get_var_type(var_id)
                .cloned()
                .or_else(|| self.get_base_var_type(var_id).cloned())
                .or_else(|| expr.ty.clone())
                .unwrap_or(Type::Any),
            SeedMode::Prescan(param_types) => {
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
            SeedMode::Lowering => self
                .codegen
                .block_narrowed_locals
                .get(var_id)
                .map(|info| info.narrowed_ty.clone())
                .or_else(|| self.get_var_type(var_id).cloned())
                .or_else(|| self.get_base_var_type(var_id).cloned())
                .or_else(|| expr.ty.clone())
                .unwrap_or(Type::Any),
        }
    }

    /// Catch-all for arms not handled explicitly. Planning/Prescan return
    /// the HIR-annotated type (`expr.ty`), falling back to `Any`. The
    /// Lowering catch-all (a cache lookup) is handled in [`Self::seed_sub`]
    /// before `arm_dispatch` is reached — non-flow kinds never enter
    /// `arm_dispatch` in Lowering mode — so this helper is mode-agnostic.
    fn seed_catchall(&self, expr: &hir::Expr) -> Type {
        expr.ty.clone().unwrap_or(Type::Any)
    }

    /// `IfExpr` arm. Prescan builds per-branch isinstance *overlays* and
    /// recurses with the narrowed param-type map on each branch; Planning
    /// (and the structurally-unreachable Lowering case) narrows the base
    /// var type via `extract_simple_isinstance_narrowing` with no overlay.
    /// Both apply `x if isinstance(x, T) else …` narrowing so the pattern
    /// `other = other if isinstance(other, Value) else Value(other)` infers
    /// `T` instead of widening to `Any` (§G.13).
    fn seed_if_expr(
        &self,
        cond: hir::ExprId,
        then_val: hir::ExprId,
        else_val: hir::ExprId,
        module: &hir::Module,
        mode: SeedMode<'_>,
    ) -> Type {
        if let SeedMode::Prescan(param_types) = mode {
            let cond_expr = &module.exprs[cond];
            let (then_ty, else_ty) = if let Some((then_overlay, else_overlay)) =
                self.build_isinstance_branch_overlays(cond_expr, module, param_types)
            {
                (
                    self.seed_sub(then_val, module, SeedMode::Prescan(Some(&then_overlay))),
                    self.seed_sub(else_val, module, SeedMode::Prescan(Some(&else_overlay))),
                )
            } else {
                (
                    self.seed_sub(then_val, module, mode),
                    self.seed_sub(else_val, module, mode),
                )
            };
            return helpers::union_or_any(then_ty, else_ty);
        }
        // Planning (and the unreachable Lowering case): narrow the base
        // var type, no overlay.
        let cond_expr = &module.exprs[cond];
        let narrow = self.extract_simple_isinstance_narrowing(cond_expr, module, None);
        match narrow {
            Some((var_id, then_narrow, else_narrow)) => {
                let then_expr = &module.exprs[then_val];
                let else_expr = &module.exprs[else_val];
                let then_ty = if matches!(&then_expr.kind, hir::ExprKind::Var(v) if *v == var_id) {
                    then_narrow
                } else {
                    self.seed_sub(then_val, module, mode)
                };
                let else_ty = if matches!(&else_expr.kind, hir::ExprKind::Var(v) if *v == var_id) {
                    else_narrow
                } else {
                    self.seed_sub(else_val, module, mode)
                };
                helpers::union_or_any(then_ty, else_ty)
            }
            None => {
                let then_ty = self.seed_sub(then_val, module, mode);
                let else_ty = self.seed_sub(else_val, module, mode);
                helpers::union_or_any(then_ty, else_ty)
            }
        }
    }

    /// Per-mode sub-expression recursion — the ONE place the three shells
    /// differ mechanically. `arm_dispatch` calls this for every child.
    ///
    /// - **Planning**: read-only cache lookup (no writes during recursion —
    ///   they are hoisted to the `&mut` `seed_expr_type_by_id` wrapper),
    ///   compute via `arm_dispatch` on a miss. Vars are never cached, so
    ///   they always recompute through `arm_dispatch` → `seed_var`.
    ///   Eager-populate (children-before-parents) makes the read-only
    ///   lookup hit for already-visited sub-expressions → O(n).
    /// - **Prescan**: direct, non-memoized `arm_dispatch`.
    /// - **Lowering**: re-evaluate the fixed flow-set of arms via
    ///   `arm_dispatch`; read the cache (falling back to `expr.ty`) for
    ///   every other kind; coerce `Never`→`Any` at every level.
    pub(crate) fn seed_sub(
        &self,
        expr_id: hir::ExprId,
        module: &hir::Module,
        mode: SeedMode<'_>,
    ) -> Type {
        match mode {
            SeedMode::Planning => {
                let expr = &module.exprs[expr_id];
                if !matches!(expr.kind, hir::ExprKind::Var(_)) {
                    if let Some(cached) = self.lowering_seed_info.lookup(expr_id) {
                        return cached.clone();
                    }
                }
                self.arm_dispatch(expr, module, mode)
            }
            SeedMode::Prescan(_) => {
                let expr = &module.exprs[expr_id];
                self.arm_dispatch(expr, module, mode)
            }
            SeedMode::Lowering => {
                let expr = &module.exprs[expr_id];
                let raw = if Self::is_lowering_flow_kind(&expr.kind) {
                    self.arm_dispatch(expr, module, mode)
                } else {
                    self.lowering_seed_info
                        .lookup(expr_id)
                        .cloned()
                        .or_else(|| expr.ty.clone())
                        .unwrap_or(Type::Any)
                };
                coerce_lowering_seed(raw)
            }
        }
    }

    /// The fixed set of `ExprKind`s that the Lowering shell re-evaluates
    /// against the current narrowing state (everything else reads the
    /// cache). Mirrors the historical explicit arms of `seed_expr_type`.
    fn is_lowering_flow_kind(kind: &hir::ExprKind) -> bool {
        matches!(
            kind,
            hir::ExprKind::Var(_)
                | hir::ExprKind::Int(_)
                | hir::ExprKind::Float(_)
                | hir::ExprKind::Bool(_)
                | hir::ExprKind::Str(_)
                | hir::ExprKind::Bytes(_)
                | hir::ExprKind::None
                | hir::ExprKind::TypeRef(_)
                | hir::ExprKind::Attribute { .. }
                | hir::ExprKind::Slice { .. }
                | hir::ExprKind::Index { .. }
                | hir::ExprKind::BuiltinCall { .. }
        )
    }
}

// =============================================================================
// Pre-scan path: direct recursion without memoization
// =============================================================================

impl<'a> Lowering<'a> {
    /// **Public** — pre-scan (Prescan-shell) entry point with a
    /// parameter-type overlay. Use this from any `type_planning/*` walker
    /// that has pre-computed types for unassigned parameters (e.g.
    /// lambda/closure capture analysis, container refinement). Non-memoized
    /// — caller pays the full sub-expression walk on every call. Pass
    /// `&IndexMap::new()` when no overlay is needed.
    ///
    /// Thin wrapper over [`Self::arm_dispatch`] in `Prescan` mode — the
    /// single shared arm table (see module docs).
    pub(crate) fn seed_infer_expr_type(
        &self,
        expr: &hir::Expr,
        module: &hir::Module,
        param_types: &IndexMap<VarId, Type>,
    ) -> Type {
        self.arm_dispatch(expr, module, SeedMode::Prescan(Some(param_types)))
    }

    /// Extract `isinstance(var, T)` (or `not isinstance(var, T)`) narrowing
    /// info from a condition expression. Returns `(var_id, then_ty, else_ty)`
    /// with the narrowing applied. Used by the `IfExpr` arm (`seed_if_expr`)
    /// of `arm_dispatch` to give ternary branches refined types.
    ///
    /// Unlike `narrowing::extract_isinstance_info`, this helper consults the
    /// supplied `param_types` overlay first so it works during pre-scan
    /// (before `self.symbols.var_types` is populated for the function
    /// currently being analysed).
    pub(crate) fn extract_simple_isinstance_narrowing(
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
        // §1.4u-b: in Planning mode this helper is reached from
        // `seed_if_expr`, which must be free of `symbols.var_types` reads so
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

    /// Build per-branch type overlays for an IfExpr whose condition might be
    /// `isinstance(var, T)` or `not isinstance(var, T)`.
    ///
    /// Returns `Some((then_overlay, else_overlay))` where `then_overlay` has
    /// `var` narrowed to the positive-isinstance type and `else_overlay` to
    /// the negative. Returns `None` when the condition is not a recognised
    /// isinstance pattern; callers should pass `current` unchanged to both
    /// branches in that case.
    ///
    /// Single DRY source for the overlay-building half of the
    /// isinstance-narrowing IfExpr arms in `seed_if_expr` (Prescan mode),
    /// `scan_expr_for_calls`, and `scan_constructor_calls_in_expr`. Fixes to
    /// the overlay logic propagate automatically to all three callers.
    pub(crate) fn build_isinstance_branch_overlays(
        &self,
        cond: &hir::Expr,
        module: &hir::Module,
        current: Option<&IndexMap<VarId, Type>>,
    ) -> Option<(IndexMap<VarId, Type>, IndexMap<VarId, Type>)> {
        let (var_id, then_narrow, else_narrow) =
            self.extract_simple_isinstance_narrowing(cond, module, current)?;
        let base: IndexMap<VarId, Type> = current.map_or_else(IndexMap::new, |m| m.clone());
        let mut then_overlay = base.clone();
        let mut else_overlay = base;
        then_overlay.insert(var_id, then_narrow);
        else_overlay.insert(var_id, else_narrow);
        Some((then_overlay, else_overlay))
    }
}

/// Extract the element type from an iterable type.
pub(crate) fn extract_iterable_element_type(ty: &Type) -> Type {
    // Union[A, B, ...]: join the element types of all variants.
    // Empty fixed-arity tuples contribute no elements so are skipped to
    // prevent widening the join to `Any`.
    if let Type::Union(variants) = ty {
        let mut joined = Type::Never;
        for v in variants {
            if v.tuple_elems().is_some_and(|e| e.is_empty()) {
                continue;
            }
            joined = joined.join(&extract_iterable_element_type(v));
        }
        return if matches!(joined, Type::Never) {
            Type::Any
        } else {
            joined
        };
    }
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
    if let Some(elem) = ty.deque_elem() {
        return elem.clone();
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
    if let Some(elem) = ty.deque_elem() {
        return elem.clone();
    }
    match ty {
        Type::Str => Type::Str,
        Type::Bytes => Type::Int,
        Type::Iterator(elem) => (**elem).clone(),
        _ => Type::Any,
    }
}

// =============================================================================
// Shared result-computation (leaf-dispatcher) helpers
// =============================================================================
//
// Each helper takes **already-resolved sub-expression types** and produces the
// parent expression's result type. `arm_dispatch` calls these after resolving
// sub-expressions via `seed_sub(mode)`, so the leaf logic is shared across all
// three shells without sharing the sub-expression-recursion strategy.
//
// `&self` — safe to call from both the memoized (`&mut self`) Planning wrapper
// and the `&self` Prescan/Lowering wrappers.

impl<'a> Lowering<'a> {
    /// Result type of `left op right`. Prefers a class-dunder's inferred
    /// return type, falls back to the numeric-tower helper, then to the
    /// HIR-annotated `expr.ty`, then `Any`.
    ///
    /// For `Union` operands, the per-variant distribution is class-aware
    /// via `resolve_binop_type_class_aware`: a `Class[T]` variant in
    /// `Float ** Union[Self, int, float, bool]` (the polymorphic
    /// numeric-dunder seed) is DROPPED from the join when `T` defines
    /// no dispatched dunder for `op`. Without this filter, the
    /// structural "Class on side → class type" fallback in
    /// `helpers::resolve_binop_type` returns `Class[T]` for every Class
    /// variant, polluting harvested field types in autograd-style code
    /// (microgpt's `Value.data` widening to `Union[Float, Class[Value]]`,
    /// which breaks `RT_INSTANCE_GET_FIELD_F64` fast-path and reduction
    /// iter unbox in `softmax`/Adam). Direct (non-Union) Class operands
    /// keep the structural fallback so explicit user code like
    /// `obj * 2` for an arbitrary class still type-plans as the class
    /// without requiring upfront dunder presence.
    pub(crate) fn binop_result_type(
        &self,
        op: &hir::BinOp,
        left_ty: &Type,
        right_ty: &Type,
        expr: &hir::Expr,
    ) -> Type {
        if let Some(ty) = self.resolve_class_binop_return(op, left_ty, right_ty) {
            ty
        } else {
            let check = |class_id: pyaot_utils::ClassId, dunder: &str| -> bool {
                match self.get_class_info(&class_id) {
                    // Cross-module class — not in the local `class_info` map,
                    // so its dunder table is unknowable here. Mirror
                    // `class_implements_protocol`: accept unconditionally
                    // rather than dropping the Union variant (a dropped
                    // cross-module class that actually defines the dunder
                    // would mistype the binop result).
                    None => true,
                    Some(ci) => ci.get_dunder_func(dunder).is_some(),
                }
            };
            helpers::resolve_binop_type_class_aware(op, left_ty, right_ty, &check)
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
    pub(crate) fn index_result_type(
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

    /// `Closure { func, … }` → the callee function's return type, or `Any` if
    /// unknown. Checks the type-planning side map (`func_return_types.inner`)
    /// first so that generator return types updated within the fixpoint loop
    /// (by `reinfer_return_types_with_prescan`) are visible to callers like
    /// `sum(genexp)` before `populate_generator_return_types_on_funcdef` runs.
    pub(super) fn closure_result_type(
        &self,
        func_id: pyaot_utils::FuncId,
        module: &hir::Module,
    ) -> Type {
        if let Some(ty) = self.get_func_return_type(&func_id) {
            return ty.clone();
        }
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
