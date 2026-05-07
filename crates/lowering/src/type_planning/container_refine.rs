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
use pyaot_types::Type;
use pyaot_utils::VarId;

use crate::Lowering;

/// Return the element type of a list-like or set-like container, or None
/// if the type is not a recognized container.
fn elem_type_of(ty: &Type) -> Option<&Type> {
    ty.list_elem().or_else(|| ty.set_elem())
}

/// True when `ty` carries no actionable refinement signal: `Any`, `Never`, or
/// a container whose element type is itself uninformative (e.g. `list[Never]`
/// from a `[]` literal). These leak through as fake refinements when an
/// `append([])` is observed before any concrete element-typed call site.
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
            if elem_ty == Type::Any || is_uninformative_elem_type(&elem_ty) {
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
                    if elem_ty != Type::Any {
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

    /// Look through subsequent statements for method calls that reveal the element type.
    fn find_elem_type_from_usage(
        &self,
        var: VarId,
        stmts: &[hir::StmtId],
        hir_module: &hir::Module,
        overlay: &IndexMap<VarId, Type>,
    ) -> Type {
        for stmt_id in stmts {
            let stmt = &hir_module.stmts[*stmt_id];
            match &stmt.kind {
                // Stop at reassignment to the same variable — any
                // subsequent `.append` targets a different list.
                hir::StmtKind::Bind {
                    target: hir::BindingTarget::Var(target_var),
                    ..
                } if *target_var == var => {
                    return Type::Any;
                }
                // Nested closure that captures our variable — recurse into
                // the closure function's body, replacing the captured-var
                // references with the corresponding `__capture_*` param.
                // Catches the idiomatic Python pattern
                //     topo = []
                //     def build_topo(v):
                //         topo.append(v)   # captures topo from the outer scope
                //     build_topo(self)
                // where the `.append()` that reveals the element type lives
                // inside a nested function, not as a sibling of the empty-list
                // bind.
                hir::StmtKind::Bind { value, .. } => {
                    let value_expr = &hir_module.exprs[*value];
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
                            // Captured vars become the first `cap_idx`
                            // leading params (`__capture_*`) on the closure
                            // function — translate `var` to the matching
                            // capture-param VarId so the recursion keys on
                            // the right identifier inside the callee body.
                            let Some(closure_func) = hir_module.func_defs.get(closure_func_id)
                            else {
                                continue;
                            };
                            let Some(capture_param) = closure_func.params.get(cap_idx) else {
                                continue;
                            };
                            // Use the closure's own prescan overlay so
                            // `append(v)` where `v` is a closure param
                            // resolves to the nested-function-inferred
                            // type (via `infer_nested_function_param_types`)
                            // rather than `Any`.
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
                                if result != Type::Any {
                                    return result;
                                }
                            }
                        }
                    }
                    // Direct call: `f(..., var, ...)` — recurse into the callee
                    // body keyed on the corresponding param VarId. Catches the
                    // container-of-container pattern where the outer list is
                    // built on the caller side
                    //     keys = [[] for _ in range(n_layer)]
                    //     gpt(token_id, keys, values)
                    // and the inner-list mutation lives in the callee
                    //     def gpt(..., keys, values):
                    //         keys[li].append(k)
                    if let Some(ty) = self.find_elem_via_call_arg(var, value_expr, hir_module) {
                        return ty;
                    }
                }
                hir::StmtKind::Expr(expr_id) => {
                    if let Some(ty) =
                        self.extract_elem_type_from_method_call(var, *expr_id, hir_module, overlay)
                    {
                        return ty;
                    }
                    // Bare statement-position call; recurse into callee body
                    // for `gpt(..., keys, ...)` patterns where the result is
                    // discarded.
                    let expr = &hir_module.exprs[*expr_id];
                    if let Some(ty) = self.find_elem_via_call_arg(var, expr, hir_module) {
                        return ty;
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
        Type::Any
    }

    /// If `expr` (or any sub-expression) is a `Call(FuncRef, args)` and one
    /// positional arg is `Var(var)`, recurse into the callee body keyed on
    /// the corresponding param VarId. Returns the element type that the
    /// param's body usage reveals (or None). Skips unresolved (dynamic) call
    /// sites.
    ///
    /// Walks through `BuiltinCall` and other wrapper expressions so combos
    /// like `print(gpt(keys))` or `total = sum(gpt(keys))` reach `gpt`'s
    /// body via the inner Call.
    fn find_elem_via_call_arg(
        &self,
        var: VarId,
        expr: &hir::Expr,
        hir_module: &hir::Module,
    ) -> Option<Type> {
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
                                if !is_uninformative_elem_type(&result) && result != Type::Any {
                                    return Some(result);
                                }
                            }
                        }
                    }
                }
                // Recurse into args.
                for call_arg in args {
                    let hir::CallArg::Regular(arg_id) = call_arg else {
                        continue;
                    };
                    let arg_expr = &hir_module.exprs[*arg_id];
                    if let Some(ty) = self.find_elem_via_call_arg(var, arg_expr, hir_module) {
                        return Some(ty);
                    }
                }
            }
            hir::ExprKind::BuiltinCall { args, .. } => {
                for arg_id in args {
                    let arg_expr = &hir_module.exprs[*arg_id];
                    if let Some(ty) = self.find_elem_via_call_arg(var, arg_expr, hir_module) {
                        return Some(ty);
                    }
                }
            }
            hir::ExprKind::MethodCall { obj, args, .. } => {
                let obj_expr = &hir_module.exprs[*obj];
                if let Some(ty) = self.find_elem_via_call_arg(var, obj_expr, hir_module) {
                    return Some(ty);
                }
                for arg_id in args {
                    let arg_expr = &hir_module.exprs[*arg_id];
                    if let Some(ty) = self.find_elem_via_call_arg(var, arg_expr, hir_module) {
                        return Some(ty);
                    }
                }
            }
            _ => {}
        }
        None
    }

    /// Check if an expression is `var.append(expr)` / `var.insert(_, expr)` / `var.add(expr)`
    /// and return the element type from the argument.
    ///
    /// Also handles the container-of-container pattern `var[idx].append(expr)`
    /// where the outer container is `var` and the inner container is the
    /// element type — yields `list[type_of(expr)]` so the outer var refines
    /// to `list[list[T]]`. Mirrors the same set of mutator method names
    /// (`append`, `insert`, `add`, `remove`) but only applies when the
    /// receiver is a Subscript on `var`. This catches the idiomatic
    ///     keys = [[] for _ in range(n_layer)]
    ///     keys[li].append(k)
    /// where the inner empty list never has a separate binding to refine.
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
        match &obj_expr.kind {
            // Case 1: var.append(expr) — element type is type_of(expr).
            // Skip uninformative arg types (`Any`, `Never`, `list[Never]` from
            // empty literals) so the scan can keep looking for a concrete
            // signal further down — e.g. a sibling `var[idx].append(elem)` or
            // a `gpt(var)` call whose body refines the inner element type.
            hir::ExprKind::Var(v) if *v == var => {
                let ty = self.seed_infer_expr_type(arg_expr, hir_module, overlay);
                if !is_uninformative_elem_type(&ty) {
                    return Some(ty);
                }
            }
            // Case 2: var[idx].append(expr) — element type is `list[type_of(expr)]`
            // (outer container holds inner lists; inner list element type is the
            // appended value type). Reject uninformative arg types (`Any`,
            // `Never`, `list[Never]`) so we don't refine the outer type to
            // `list[list[Never]]` from a coincidental indexed empty-append —
            // the wrapping `list[..]` would conceal the missing element info.
            hir::ExprKind::Index { obj: inner_obj, .. } => {
                let inner_obj_expr = &hir_module.exprs[*inner_obj];
                if matches!(&inner_obj_expr.kind, hir::ExprKind::Var(v) if v == &var) {
                    let arg_ty = self.seed_infer_expr_type(arg_expr, hir_module, overlay);
                    if !is_uninformative_elem_type(&arg_ty) {
                        return Some(Type::list_of(arg_ty));
                    }
                }
            }
            _ => {}
        }
        None
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
