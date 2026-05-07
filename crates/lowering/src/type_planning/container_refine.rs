//! Empty container type refinement
//!
//! When `li = []` has no type annotation, the type planner infers `List(Any)`.
//! Without refinement, appending raw int Values into a List(Any) container
//! could cause type mismatches that lead to segfaults.
//!
//! This pass scans statement blocks for empty container assignments and refines
//! their element type from subsequent method calls (append, insert, add, etc.).

use indexmap::IndexMap;
use pyaot_hir as hir;
use pyaot_types::{Type, TypeLattice};
use pyaot_utils::VarId;

use crate::Lowering;

/// Return the element type of a list-like or set-like container, or None
/// if the type is not a recognized container.
fn elem_type_of(ty: &Type) -> Option<&Type> {
    ty.list_elem().or_else(|| ty.set_elem())
}

/// True when `ty` carries no actionable refinement signal: `Any`, `Never`, or
/// a container whose element type is itself uninformative (e.g. `list[Never]`
/// from a `[]` literal). Used for outer-level guards that protect already-
/// refined types and for the candidate-shape filter; the per-source-point
/// scan itself accumulates everything via lattice join, where `Never` is
/// the identity, so empty-literal sources merge harmlessly with concrete
/// ones (`Never ⊔ Float = Float`, `list[Never] ⊔ list[Float] = list[Float]`).
fn is_uninformative_elem_type(ty: &Type) -> bool {
    match ty {
        Type::Any | Type::Never => true,
        _ => match elem_type_of(ty) {
            Some(inner) => is_uninformative_elem_type(inner),
            None => match ty {
                // Empty dict literal — both K and V inferred as Never.
                Type::Generic { args, .. } if args.iter().any(is_uninformative_elem_type) => true,
                _ => false,
            },
        },
    }
}

/// If `expr` is a chain of `Index{...Var(target)}`, return the chain depth:
/// `Var(target)` → 0, `target[i]` → 1, `target[i][j]` → 2, etc. Returns
/// `None` for any node that isn't a pure subscript on `target` — including
/// `Slice` (slices preserve container rank, unlike `Index` which strips one
/// level), `BinOp`, `Call`, attribute access, etc.
///
/// The depth value drives `extract_elem_type_from_method_call`: when
/// `target[i]...[k].append(arg)` is seen, the outer container's element
/// type is `arg_ty` wrapped in `list_of` `k` times — so a 3-deep mutation
/// `var[i][j][k].append(x)` refines `var` to `list[list[list[T]]]`.
fn subscript_depth_to_var(
    expr: &hir::Expr,
    target: VarId,
    hir_module: &hir::Module,
) -> Option<usize> {
    match &expr.kind {
        hir::ExprKind::Var(v) if *v == target => Some(0),
        hir::ExprKind::Index { obj, .. } => {
            // The Index's `index` payload is whatever scalar the Python
            // bracket holds — number, name, attr lookup, even arithmetic.
            // We don't care what it is, only that the *shape* is `obj[...]`
            // — that's what guarantees one-level-of-rank reduction.
            // `Slice` is a *separate* ExprKind, so it never reaches here;
            // `var[1:2].append(x)` parses as `MethodCall { obj: Slice {...} }`
            // and the slice node bottoms this match out at `_ => None`.
            subscript_depth_to_var(&hir_module.exprs[*obj], target, hir_module).map(|d| d + 1)
        }
        _ => None,
    }
}

/// Wrap `inner` in `list_of` `depth` times. `wrap_list(T, 0) == T`,
/// `wrap_list(T, 1) == list[T]`, `wrap_list(T, 2) == list[list[T]]`.
fn wrap_list(inner: Type, depth: usize) -> Type {
    let mut ty = inner;
    for _ in 0..depth {
        ty = Type::list_of(ty);
    }
    ty
}

impl<'a> Lowering<'a> {
    /// Refine types of empty containers by scanning for subsequent method calls.
    /// Must run before lowering so that `get_var_type` returns the refined type.
    ///
    /// §1.17b-d/f — all HIR functions, including the synthetic module-init
    /// function, are scanned via their CFG blocks in allocation order. Each
    /// block is treated as a flat stmt list; "subsequent uses" are read from
    /// `block.stmts[i+1..]`.
    pub(crate) fn refine_empty_container_types(&mut self, hir_module: &hir::Module) {
        for func_id in hir_module.functions.iter() {
            if let Some(func) = hir_module.func_defs.get(func_id) {
                let overlay = self
                    .lowering_seed_info
                    .per_function_local_seed_types
                    .get(func_id)
                    .cloned()
                    .unwrap_or_default();
                // Flatten all blocks in CFG-insertion order so refinement
                // can cross block boundaries — comprehension desugarings emit
                // `__comp = []` in the entry block and `__comp.append(p)` in
                // the for-loop body block; without flattening, the empty-list
                // and the `.append()` live in different blocks and refinement
                // misses the element type, leaving the synthetic var typed as
                // `list[Never]`. Block insertion order is HIR construction
                // order (a pre-order DFS of source-form statements), so the
                // flattened sequence preserves intra-function "after this
                // bind" semantics that the reassignment heuristic relies on.
                let flattened: Vec<hir::StmtId> = func
                    .blocks
                    .values()
                    .flat_map(|b| b.stmts.iter().copied())
                    .collect();
                self.refine_empty_containers_in_block(&flattened, hir_module, &overlay);
                self.refine_indexed_var_types_in_func(func_id, &flattened, hir_module, &overlay);
            }
        }
    }

    /// Refine types of unannotated container-typed variables (function params
    /// and locally-bound vars) by scanning the body for the
    /// `var[idx].append(arg)` pattern (container-of-container indexed
    /// mutation), or by recursing into a callee body when `var` is forwarded
    /// to a function.
    ///
    /// The motivating case is built on the caller side and mutated in the
    /// callee:
    ///     keys = [[] for _ in range(n_layer)]   # caller — outer list[list[?]]
    ///     gpt(token_id, keys, values)            # forwards keys to callee
    ///     ...
    ///     def gpt(..., keys, values):
    ///         keys[li].append(k)                 # inner mutation
    /// Empty-container refinement only handles `var = []` literal binds, so
    /// the listcomp-built outer list and the callee's param both miss out.
    /// This pass closes both gaps: `find_elem_type_from_usage` already knows
    /// how to follow callee bodies (via `find_elem_via_call_arg`) and how to
    /// recognize `var[idx].append(arg)` (Case 2 in
    /// `extract_elem_type_from_method_call`).
    ///
    /// We write to `refined_container_types[var]` only — the harvester
    /// remains the source of truth for call-site arg-type inference, and
    /// MIR param-type selection in `function_lowering` reads
    /// `refined_container_types` before falling through to harvester hints.
    fn refine_indexed_var_types_in_func(
        &mut self,
        func_id: &pyaot_utils::FuncId,
        stmts: &[hir::StmtId],
        hir_module: &hir::Module,
        overlay: &IndexMap<VarId, Type>,
    ) {
        let Some(func) = hir_module.func_defs.get(func_id) else {
            return;
        };
        let capture_count = self
            .get_closure_capture_types(func_id)
            .map(|v| v.len())
            .unwrap_or(0);

        // Collect candidate vars: unannotated params plus all locally-bound
        // vars in this function. We snapshot now because we mutate self below.
        let mut candidates: Vec<VarId> = func
            .params
            .iter()
            .skip(capture_count)
            .filter(|p| p.ty.is_none())
            .map(|p| p.var)
            .collect();
        for stmt_id in stmts {
            let stmt = &hir_module.stmts[*stmt_id];
            if let hir::StmtKind::Bind {
                target: hir::BindingTarget::Var(target_var),
                type_hint: None,
                ..
            } = &stmt.kind
            {
                candidates.push(*target_var);
            }
        }
        candidates.sort_by_key(|v| v.0);
        candidates.dedup();

        for var_id in candidates {
            // Determine current type: refined > overlay > Any.
            let prev = self
                .lowering_seed_info
                .refined_container_types
                .get(&var_id)
                .cloned();
            // Skip if already refined to a concrete element type.
            if let Some(prev_ty) = &prev {
                if let Some(elem) = elem_type_of(prev_ty) {
                    if !is_uninformative_elem_type(elem) {
                        continue;
                    }
                }
            }
            // Only refine if the current best-known type is a container
            // (`list[..]`, `set[..]`) with an uninformative element; otherwise
            // the indexed-mutation refinement would invent a container shape
            // around an unrelated scalar.
            let current_overlay_ty = overlay.get(&var_id).cloned();
            let probe_ty = prev.as_ref().or(current_overlay_ty.as_ref());
            let (is_list_like, is_set_like) = match probe_ty {
                Some(t) => (t.list_elem().is_some(), t.set_elem().is_some()),
                None => (false, false),
            };
            if !(is_list_like || is_set_like) {
                continue;
            }

            // For locally-bound vars, scan starts AFTER the binding stmt —
            // otherwise `find_elem_type_from_usage` immediately returns `Any`
            // when it sees `var = ...` as a reassignment. Params don't have
            // a binding so we scan from the start.
            let scan_start = stmts
                .iter()
                .position(|stmt_id| {
                    let stmt = &hir_module.stmts[*stmt_id];
                    matches!(
                        &stmt.kind,
                        hir::StmtKind::Bind {
                            target: hir::BindingTarget::Var(t),
                            ..
                        } if *t == var_id
                    )
                })
                .map(|i| i + 1)
                .unwrap_or(0);
            let elem_ty =
                self.find_elem_type_from_usage(var_id, &stmts[scan_start..], hir_module, overlay);
            // `find_elem_type_from_usage` accumulates via lattice join with
            // `Never` as identity, so an empty accumulator means "no signal";
            // `Any` means a top-level absorbing source-point poisoned the
            // accumulator (rare — `extract_elem_type_from_method_call`
            // rejects `Any` arg types). Skip both — neither carries useful
            // refinement information.
            if elem_ty == Type::Any
                || elem_ty == Type::Never
                || is_uninformative_elem_type(&elem_ty)
            {
                continue;
            }
            let refined = if is_set_like {
                Type::set_of(elem_ty)
            } else {
                Type::list_of(elem_ty)
            };
            if Some(&refined) == prev.as_ref() {
                continue;
            }
            self.lowering_seed_info
                .refined_container_types
                .insert(var_id, refined);
        }
    }

    /// Scan a flat statement block for `var = []` followed by `var.append(expr)`
    /// and refine the variable's type.
    fn refine_empty_containers_in_block(
        &mut self,
        stmts: &[hir::StmtId],
        hir_module: &hir::Module,
        overlay: &IndexMap<VarId, Type>,
    ) {
        for (i, stmt_id) in stmts.iter().enumerate() {
            let stmt = &hir_module.stmts[*stmt_id];

            // Look for: var = [] / {} / set() (no type hint, empty container)
            let empty_container = match &stmt.kind {
                hir::StmtKind::Bind {
                    target: hir::BindingTarget::Var(target_var),
                    value,
                    type_hint: None,
                } => Some((*target_var, *value)),
                _ => None,
            };
            if let Some((target, value)) = empty_container {
                let expr = &hir_module.exprs[value];
                let is_empty_list =
                    matches!(&expr.kind, hir::ExprKind::List(elems) if elems.is_empty());
                let is_empty_set =
                    matches!(&expr.kind, hir::ExprKind::Set(elems) if elems.is_empty());
                let is_empty_dict =
                    matches!(&expr.kind, hir::ExprKind::Dict(pairs) if pairs.is_empty());

                if is_empty_dict {
                    // Scan for d[key] = value assignments to infer key/value types
                    let (key_ty, val_ty) = self.find_dict_types_from_usage(
                        target,
                        &stmts[i + 1..],
                        hir_module,
                        overlay,
                    );
                    if key_ty != Type::Any || val_ty != Type::Any {
                        let refined = Type::dict_of(key_ty, val_ty);
                        self.lowering_seed_info
                            .refined_container_types
                            .insert(target, refined);
                    }
                } else if is_empty_list || is_empty_set {
                    // Scan subsequent statements for method calls on this variable
                    let elem_ty = self.find_elem_type_from_usage(
                        target,
                        &stmts[i + 1..],
                        hir_module,
                        overlay,
                    );
                    // After the lattice-join rewrite, no signal returns
                    // `Never`; only an unrelated absorbing observation
                    // returns `Any`. Either way, don't refine.
                    if elem_ty != Type::Any && elem_ty != Type::Never {
                        let refined = if is_empty_list {
                            Type::list_of(elem_ty)
                        } else {
                            Type::set_of(elem_ty)
                        };
                        // Store in refined_container_types which persists across function lowerings
                        self.lowering_seed_info
                            .refined_container_types
                            .insert(target, refined);
                    }
                }
            }
        }
    }

    /// Walk the statement list and accumulate every observed element-type
    /// signal for `var` via `TypeLattice::join`. Source-points include:
    ///
    /// - `var[...]*.{append,insert,add,remove}(arg)` mutators at any
    ///   subscript depth (via `extract_elem_type_from_method_call` →
    ///   `subscript_depth_to_var` + `wrap_list`).
    /// - Closures that capture `var` — recurse into the closure body keyed
    ///   on the corresponding `__capture_*` param.
    /// - Direct / nested calls forwarding `var` to a callee — recurse into
    ///   the callee body via `find_elem_via_call_arg`.
    ///
    /// Returns `Type::Never` (lattice bottom) when no signal is found.
    /// `Never` is the join identity, so callers can compare against it as
    /// "no information"; concrete signals merge cleanly via the lattice
    /// (`list[Never] ⊔ list[Float] = list[Float]`, `Bool ⊔ Int = Int`,
    /// etc.).
    ///
    /// Reassignment (`var = ...`) breaks the loop — past the rebind the var
    /// holds a different value whose element-type observations would be
    /// unrelated and pollute the accumulator. A future SSA-based pass could
    /// recover those by Phi-merging per-version observations; for now we
    /// stop conservatively.
    fn find_elem_type_from_usage(
        &self,
        var: VarId,
        stmts: &[hir::StmtId],
        hir_module: &hir::Module,
        overlay: &IndexMap<VarId, Type>,
    ) -> Type {
        let mut accum = Type::Never;
        for stmt_id in stmts {
            let stmt = &hir_module.stmts[*stmt_id];
            match &stmt.kind {
                // Reassignment ends the meaningful scan window for `var`.
                hir::StmtKind::Bind {
                    target: hir::BindingTarget::Var(target_var),
                    ..
                } if *target_var == var => {
                    break;
                }
                hir::StmtKind::Bind { value, .. } => {
                    let value_expr = &hir_module.exprs[*value];
                    // Closure capture: `var` flows into a nested function via
                    // `__capture_<n>`. Recurse into every block of the closure
                    // body using the closure's own overlay so its params are
                    // typed (e.g. `def f(v): topo.append(v)` where `topo`
                    // captures and `v` is a harvester-typed param).
                    if let hir::ExprKind::Closure {
                        func: closure_func_id,
                        captures,
                    } = &value_expr.kind
                    {
                        for (cap_idx, cap_id) in captures.iter().enumerate() {
                            let cap_expr = &hir_module.exprs[*cap_id];
                            if !matches!(&cap_expr.kind, hir::ExprKind::Var(v) if *v == var) {
                                continue;
                            }
                            let Some(closure_func) = hir_module.func_defs.get(closure_func_id)
                            else {
                                continue;
                            };
                            let Some(capture_param) = closure_func.params.get(cap_idx) else {
                                continue;
                            };
                            let closure_overlay = self
                                .lowering_seed_info
                                .per_function_local_seed_types
                                .get(closure_func_id)
                                .cloned()
                                .unwrap_or_default();
                            for block in closure_func.blocks.values() {
                                let result = self.find_elem_type_from_usage(
                                    capture_param.var,
                                    &block.stmts,
                                    hir_module,
                                    &closure_overlay,
                                );
                                accum = accum.join(&result);
                            }
                        }
                    }
                    // Direct / nested call forwarding `var` to a callee.
                    if let Some(ty) = self.find_elem_via_call_arg(var, value_expr, hir_module) {
                        accum = accum.join(&ty);
                    }
                }
                hir::StmtKind::Expr(expr_id) => {
                    if let Some(ty) =
                        self.extract_elem_type_from_method_call(var, *expr_id, hir_module, overlay)
                    {
                        accum = accum.join(&ty);
                    }
                    let expr = &hir_module.exprs[*expr_id];
                    if let Some(ty) = self.find_elem_via_call_arg(var, expr, hir_module) {
                        accum = accum.join(&ty);
                    }
                }
                hir::StmtKind::Return(_)
                | hir::StmtKind::Break
                | hir::StmtKind::Continue
                | hir::StmtKind::Raise { .. }
                | hir::StmtKind::Pass
                | hir::StmtKind::Assert { .. }
                | hir::StmtKind::IndexDelete { .. }
                | hir::StmtKind::IterAdvance { .. }
                | hir::StmtKind::IterSetup { .. } => {}
            }
        }
        accum
    }

    /// Lattice-join every element-type signal reachable from `expr` for
    /// occurrences of `Var(var)` passed to a callee.
    ///
    /// Walks through `Call`, `BuiltinCall`, and `MethodCall` wrappers (so
    /// `print(gpt(keys))` and `total = sum(gpt(keys))` both reach `gpt`'s
    /// body via the inner `Call`). For each FuncRef call site, finds args
    /// matching `Var(var)` and recurses into the callee's body keyed on the
    /// corresponding param VarId. All callee bodies and all args are
    /// scanned, with results merged via `join`. Returns `None` only when no
    /// concrete signal was found (the accumulator never left `Never`).
    ///
    /// Skips unresolved (dynamic) call sites — Var-typed callable, attribute
    /// dispatch through VTable, etc. Those need a separate method-dispatch
    /// pass to resolve which FuncId is invoked.
    fn find_elem_via_call_arg(
        &self,
        var: VarId,
        expr: &hir::Expr,
        hir_module: &hir::Module,
    ) -> Option<Type> {
        let mut accum = Type::Never;
        let mut found = false;
        match &expr.kind {
            hir::ExprKind::Call { func, args, .. } => {
                let func_expr = &hir_module.exprs[*func];
                if let hir::ExprKind::FuncRef(fid) = &func_expr.kind {
                    let fid = *fid;
                    if let Some(callee) = hir_module.func_defs.get(&fid) {
                        let capture_count = self
                            .get_closure_capture_types(&fid)
                            .map(|v| v.len())
                            .unwrap_or(0);
                        for (arg_idx, call_arg) in args.iter().enumerate() {
                            let hir::CallArg::Regular(arg_id) = call_arg else {
                                continue;
                            };
                            let arg_expr = &hir_module.exprs[*arg_id];
                            if !matches!(&arg_expr.kind, hir::ExprKind::Var(v) if *v == var) {
                                continue;
                            }
                            let param_slot = capture_count + arg_idx;
                            let Some(param) = callee.params.get(param_slot) else {
                                continue;
                            };
                            let callee_overlay = self
                                .lowering_seed_info
                                .per_function_local_seed_types
                                .get(&fid)
                                .cloned()
                                .unwrap_or_default();
                            for block in callee.blocks.values() {
                                let result = self.find_elem_type_from_usage(
                                    param.var,
                                    &block.stmts,
                                    hir_module,
                                    &callee_overlay,
                                );
                                if result != Type::Any && result != Type::Never {
                                    accum = accum.join(&result);
                                    found = true;
                                }
                            }
                        }
                    }
                }
                // Recurse into nested call args (e.g. `print(gpt(keys))`).
                for call_arg in args {
                    let hir::CallArg::Regular(arg_id) = call_arg else {
                        continue;
                    };
                    let arg_expr = &hir_module.exprs[*arg_id];
                    if let Some(ty) = self.find_elem_via_call_arg(var, arg_expr, hir_module) {
                        accum = accum.join(&ty);
                        found = true;
                    }
                }
            }
            hir::ExprKind::BuiltinCall { args, .. } => {
                for arg_id in args {
                    let arg_expr = &hir_module.exprs[*arg_id];
                    if let Some(ty) = self.find_elem_via_call_arg(var, arg_expr, hir_module) {
                        accum = accum.join(&ty);
                        found = true;
                    }
                }
            }
            hir::ExprKind::MethodCall { obj, args, .. } => {
                let obj_expr = &hir_module.exprs[*obj];
                if let Some(ty) = self.find_elem_via_call_arg(var, obj_expr, hir_module) {
                    accum = accum.join(&ty);
                    found = true;
                }
                for arg_id in args {
                    let arg_expr = &hir_module.exprs[*arg_id];
                    if let Some(ty) = self.find_elem_via_call_arg(var, arg_expr, hir_module) {
                        accum = accum.join(&ty);
                        found = true;
                    }
                }
            }
            _ => {}
        }
        if found && accum != Type::Any {
            Some(accum)
        } else {
            None
        }
    }

    /// Check if an expression is a mutator method call on a subscript chain
    /// rooted at `var` (depth 0 = `var.append(expr)`, depth 1 =
    /// `var[i].append(expr)`, depth 2 = `var[i][j].append(expr)`, …) and
    /// return the corresponding outer-element type.
    ///
    /// At depth `k` the appended value `expr` lives `k` levels deep below
    /// the outer container, so `var`'s element type is `wrap_list(arg_ty, k)`
    /// — yielding refinements like `list[T]` (depth 1), `list[list[T]]`
    /// (depth 2), `list[list[list[T]]]` (depth 3).
    ///
    /// Returns `None` when the receiver isn't a pure subscript chain on
    /// `var` (e.g. attribute access, slice, computed receiver), when the
    /// method isn't a recognized mutator, or when `arg_ty` is `Any`. We
    /// intentionally allow `Never` and `list[Never]` (from empty literals)
    /// to propagate — the caller folds source-points via lattice join, where
    /// `Never` is the identity and `list[Never] ⊔ list[T] = list[T]`. This
    /// preserves caller-side refinement when the only concrete signal lives
    /// further down the body or in a forwarded callee.
    fn extract_elem_type_from_method_call(
        &self,
        var: VarId,
        expr_id: hir::ExprId,
        hir_module: &hir::Module,
        overlay: &IndexMap<VarId, Type>,
    ) -> Option<Type> {
        let expr = &hir_module.exprs[expr_id];
        let hir::ExprKind::MethodCall {
            obj, method, args, ..
        } = &expr.kind
        else {
            return None;
        };

        let method_name = self.interner.resolve(*method);
        let value_arg_idx = match method_name {
            "append" | "add" | "remove" => Some(0),
            "insert" => Some(1), // insert(index, value)
            _ => None,
        };
        let idx = value_arg_idx?;
        let arg_id = args.get(idx)?;
        let arg_expr = &hir_module.exprs[*arg_id];

        let obj_expr = &hir_module.exprs[*obj];
        let depth = subscript_depth_to_var(obj_expr, var, hir_module)?;
        let arg_ty = self.seed_infer_expr_type(arg_expr, hir_module, overlay);
        // Reject `Any` because lattice join treats it as top — a single
        // `Any`-typed source-point would absorb every concrete sibling and
        // collapse the accumulator. Allow `Never` / `list[Never]`: they're
        // join-identity, so they don't pollute concrete observations.
        if arg_ty == Type::Any {
            return None;
        }
        Some(wrap_list(arg_ty, depth))
    }

    /// Look through subsequent statements for dict index assignments (`d[key] = value`)
    /// that reveal the key and value types.
    fn find_dict_types_from_usage(
        &self,
        var: VarId,
        stmts: &[hir::StmtId],
        hir_module: &hir::Module,
        overlay: &IndexMap<VarId, Type>,
    ) -> (Type, Type) {
        for stmt_id in stmts {
            let stmt = &hir_module.stmts[*stmt_id];
            match &stmt.kind {
                hir::StmtKind::Bind {
                    target: hir::BindingTarget::Index { obj, index, .. },
                    value,
                    ..
                } => {
                    let obj_expr = &hir_module.exprs[*obj];
                    if matches!(&obj_expr.kind, hir::ExprKind::Var(v) if *v == var) {
                        let key_ty = self.seed_infer_expr_type(
                            &hir_module.exprs[*index],
                            hir_module,
                            overlay,
                        );
                        let val_ty = self.seed_infer_expr_type(
                            &hir_module.exprs[*value],
                            hir_module,
                            overlay,
                        );
                        if key_ty != Type::Any && val_ty != Type::Any {
                            return (key_ty, val_ty);
                        }
                    }
                }
                // Stop at reassignment to the same variable
                hir::StmtKind::Bind {
                    target: hir::BindingTarget::Var(target_var),
                    ..
                } if *target_var == var => {
                    return (Type::Any, Type::Any);
                }
                hir::StmtKind::Bind { .. }
                | hir::StmtKind::Expr(_)
                | hir::StmtKind::Return(_)
                | hir::StmtKind::Break
                | hir::StmtKind::Continue
                | hir::StmtKind::Raise { .. }
                | hir::StmtKind::Pass
                | hir::StmtKind::Assert { .. }
                | hir::StmtKind::IndexDelete { .. }
                | hir::StmtKind::IterAdvance { .. }
                | hir::StmtKind::IterSetup { .. } => {}
            }
        }
        (Type::Any, Type::Any)
    }
}
