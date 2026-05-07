//! Empty container type refinement
//!
//! When `li = []` has no type annotation, the type planner infers `List(Any)`.
//! Without refinement, appending raw int Values into a List(Any) container
//! could cause type mismatches that lead to segfaults.
//!
//! This pass scans statement blocks for empty container assignments and refines
//! their element type from subsequent method calls (append, insert, add, etc.).

use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;
use pyaot_hir as hir;
use pyaot_types::{Type, TypeLattice};
use pyaot_utils::{FuncId, VarId};

use crate::Lowering;

/// Build a map from `VarId` to `(callee FuncId, capture count)` by scanning
/// all `Bind { Var, Closure | FuncRef }` statements in the module. Mirrors
/// the same pattern used by `infer_nested_function_param_types_inner` in
/// `closure_scan.rs` (last-bind-wins on rebinds).
///
/// Used by `find_elem_via_call_arg` to resolve `Call { func: Var(v), args }`
/// to a concrete callee body, so caller-side container refinement can chase
/// element-type signals through indirected calls.
fn build_var_to_func_map(hir_module: &hir::Module) -> HashMap<VarId, (FuncId, usize)> {
    let mut map: HashMap<VarId, (FuncId, usize)> = HashMap::new();
    for (_stmt_id, stmt) in hir_module.stmts.iter() {
        let hir::StmtKind::Bind { target, value, .. } = &stmt.kind else {
            continue;
        };
        let hir::BindingTarget::Var(var_id) = target else {
            continue;
        };
        let value_expr = &hir_module.exprs[*value];
        match &value_expr.kind {
            hir::ExprKind::Closure { func, captures } => {
                map.insert(*var_id, (*func, captures.len()));
            }
            hir::ExprKind::FuncRef(func_id) => {
                map.insert(*var_id, (*func_id, 0));
            }
            _ => {}
        }
    }
    map
}

/// Resolution result for `obj.method(...)` — which FuncId backs the method
/// body, and how many leading params it consumes for `self`/`cls` (so the
/// caller can map call-site arg indices to callee param slots).
#[derive(Clone, Copy)]
pub(crate) struct MethodResolution {
    pub(crate) func_id: FuncId,
    /// Slot offset of the first user-visible (non-self/cls) param.
    /// 0 for static methods, 1 for instance/dunder/classmethod.
    pub(crate) self_offset: usize,
}

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
    /// Pre-scan every `MethodCall` site in the module and accumulate a
    /// per-FuncId map of `param.var → joined caller-arg type`. The
    /// harvester deliberately doesn't visit method-call args (committing
    /// them as `lambda_param_type_hints` breaks dunders like `__exit__`
    /// whose arg shape varies between code paths), but refinement still
    /// needs the data: without it `Cache.add(self, store, k)`'s `k` stays
    /// `Any` inside the body, and `store.append(k)` fails to refine
    /// `store`'s element type.
    ///
    /// Output is consumed only by the refinement pass — layered onto each
    /// function's `overlay` for the duration of the scan, never written
    /// back to `lambda_param_type_hints` or `per_function_local_seed_types`,
    /// so it can't influence MIR param type selection or break dunders.
    pub(crate) fn build_method_arg_seeds(
        &self,
        hir_module: &hir::Module,
    ) -> HashMap<FuncId, IndexMap<VarId, Type>> {
        let mut out: HashMap<FuncId, IndexMap<VarId, Type>> = HashMap::new();
        for (_expr_id, expr) in hir_module.exprs.iter() {
            let hir::ExprKind::MethodCall {
                obj, method, args, ..
            } = &expr.kind
            else {
                continue;
            };
            let obj_expr = &hir_module.exprs[*obj];
            let obj_ty = self.seed_infer_expr_type(obj_expr, hir_module, &IndexMap::new());
            let Some(MethodResolution {
                func_id,
                self_offset,
            }) = self.resolve_method_func(&obj_ty, *method)
            else {
                continue;
            };
            let Some(callee) = hir_module.func_defs.get(&func_id) else {
                continue;
            };
            let entry = out.entry(func_id).or_default();
            for (arg_idx, arg_id) in args.iter().enumerate() {
                let arg_expr = &hir_module.exprs[*arg_id];
                let arg_ty = self.seed_infer_expr_type(arg_expr, hir_module, &IndexMap::new());
                if arg_ty == Type::Any || arg_ty == Type::Never {
                    continue;
                }
                let param_slot = self_offset + arg_idx;
                let Some(param) = callee.params.get(param_slot) else {
                    continue;
                };
                // Multiple call sites: lattice-join across observations
                // so a stable concrete-type wins over Any/Never. Mirrors
                // `join_nested_arg_ty` semantics for paramater types.
                let joined = match entry.get(&param.var) {
                    Some(existing) => existing.join(&arg_ty),
                    None => arg_ty,
                };
                entry.insert(param.var, joined);
            }
        }
        out
    }

    /// Resolve `obj.method(...)` to the FuncId backing the method body and
    /// the leading-param offset (`self_offset`) that separates `self`/`cls`
    /// slots from user-visible positional args.
    ///
    /// Mirrors the priority order in `lower_class_method_call`
    /// (`expressions/access/method/class.rs:82-248`):
    ///
    /// 1. `static_methods` — `@staticmethod` bodies, no implicit first param
    ///    (offset 0).
    /// 2. `class_methods` — `@classmethod` bodies, `cls` is first (offset 1).
    /// 3. `dunder_methods` — `__add__`/`__eq__`/etc, `self` is first
    ///    (offset 1).
    /// 4. `method_funcs` — regular instance methods, `self` is first
    ///    (offset 1). Inherited methods are already merged in by the
    ///    topological build in `class_metadata.rs`, so no parent-chain walk
    ///    is needed here.
    ///
    /// Returns `None` when:
    /// - `obj_ty` is not a user-class (`Any`/`HeapAny`/`Union`/`Iterator`/
    ///   builtin containers — `get_class_info` is `None`).
    /// - The class is cross-module (no entry in our `class_info` map).
    /// - The method name isn't found in any of the four registries.
    pub(crate) fn resolve_method_func(
        &self,
        obj_ty: &Type,
        method: pyaot_utils::InternedString,
    ) -> Option<MethodResolution> {
        let class_id = match obj_ty {
            Type::Class { class_id, .. } => *class_id,
            // `Generic { base }` covers user-defined generic classes
            // (`Stack[T]`); builtin container ClassIds (LIST/DICT/SET/TUPLE)
            // simply have no `class_info` entry, so the `?` below returns
            // `None` and we fall through cleanly.
            Type::Generic { base, .. } => *base,
            _ => return None,
        };
        let info = self.get_class_info(&class_id)?;

        if let Some(&fid) = info.static_methods.get(&method) {
            return Some(MethodResolution {
                func_id: fid,
                self_offset: 0,
            });
        }
        if let Some(&fid) = info.class_methods.get(&method) {
            return Some(MethodResolution {
                func_id: fid,
                self_offset: 1,
            });
        }
        let method_str = self.interner.resolve(method);
        if let Some(fid) = info.get_dunder_func(method_str) {
            return Some(MethodResolution {
                func_id: fid,
                self_offset: 1,
            });
        }
        if let Some(&fid) = info.method_funcs.get(&method) {
            return Some(MethodResolution {
                func_id: fid,
                self_offset: 1,
            });
        }
        None
    }

    /// Refine types of empty containers by scanning for subsequent method calls.
    /// Must run before lowering so that `get_var_type` returns the refined type.
    ///
    /// §1.17b-d/f — all HIR functions, including the synthetic module-init
    /// function, are scanned via their CFG blocks in allocation order. Each
    /// block is treated as a flat stmt list; "subsequent uses" are read from
    /// `block.stmts[i+1..]`.
    pub(crate) fn refine_empty_container_types(&mut self, hir_module: &hir::Module) {
        // Build a module-wide var-to-FuncId map once per refine pass.
        // Used by `find_elem_via_call_arg` to resolve `Call(Var(v), ...)`
        // — closures/funcrefs assigned to a variable. Pattern mirrors
        // `infer_nested_function_param_types_inner` in `closure_scan.rs`.
        let var_to_func = build_var_to_func_map(hir_module);
        // Build a per-FuncId map of `param.var → caller-arg type` from
        // every `MethodCall` site in the module. The harvester
        // (`infer_nested_function_param_types`) doesn't visit method-call
        // arg types because committing them to `lambda_param_type_hints`
        // breaks dunder methods like `__exit__(self, exc_type, exc_val,
        // exc_tb)` whose arg-shape varies between no-error path
        // (`(None, None, None)`) and exception path (`(type, val, tb)`).
        // We collect the same data here for refinement-only use, so
        // callee body scans see concrete param types from caller-arg
        // inference without affecting MIR param type selection.
        let method_arg_seeds = self.build_method_arg_seeds(hir_module);
        for func_id in hir_module.functions.iter() {
            if let Some(func) = hir_module.func_defs.get(func_id) {
                let mut overlay = self
                    .lowering_seed_info
                    .per_function_local_seed_types
                    .get(func_id)
                    .cloned()
                    .unwrap_or_default();
                // Layer method-arg-seeded types onto the function's
                // overlay (only for non-Any/Never values, and only when
                // the existing entry is uninformative). This lets
                // `extract_elem_type_from_method_call` see e.g. `k: Int`
                // inside `Cache.add` body, even before harvester /
                // prescan would converge.
                if let Some(seeds) = method_arg_seeds.get(func_id) {
                    for (var_id, ty) in seeds {
                        let need_seed = match overlay.get(var_id) {
                            Some(existing) => is_uninformative_elem_type(existing),
                            None => true,
                        };
                        if need_seed {
                            overlay.insert(*var_id, ty.clone());
                        }
                    }
                }
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
                self.refine_empty_containers_in_block(
                    &flattened,
                    hir_module,
                    &overlay,
                    &var_to_func,
                );
                self.refine_indexed_var_types_in_func(
                    func_id,
                    &flattened,
                    hir_module,
                    &overlay,
                    &var_to_func,
                );
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
        var_to_func: &HashMap<VarId, (FuncId, usize)>,
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
            let mut visited: HashSet<FuncId> = HashSet::new();
            let elem_ty = self.find_elem_type_from_usage(
                var_id,
                &stmts[scan_start..],
                hir_module,
                overlay,
                var_to_func,
                &mut visited,
            );
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
        var_to_func: &HashMap<VarId, (FuncId, usize)>,
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
                    let mut visited: HashSet<FuncId> = HashSet::new();
                    let elem_ty = self.find_elem_type_from_usage(
                        target,
                        &stmts[i + 1..],
                        hir_module,
                        overlay,
                        var_to_func,
                        &mut visited,
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
        var_to_func: &HashMap<VarId, (FuncId, usize)>,
        visited: &mut HashSet<FuncId>,
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
                            // Cycle-guard for closures too — a closure body
                            // could indirectly reach itself through nested
                            // calls, leading to unbounded recursion.
                            if !visited.insert(*closure_func_id) {
                                continue;
                            }
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
                                    var_to_func,
                                    visited,
                                );
                                accum = accum.join(&result);
                            }
                            visited.remove(closure_func_id);
                        }
                    }
                    // Direct / nested call forwarding `var` to a callee.
                    if let Some(ty) = self.find_elem_via_call_arg(
                        var,
                        value_expr,
                        hir_module,
                        overlay,
                        var_to_func,
                        visited,
                    ) {
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
                    if let Some(ty) = self.find_elem_via_call_arg(
                        var,
                        expr,
                        hir_module,
                        overlay,
                        var_to_func,
                        visited,
                    ) {
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
    /// body via the inner `Call`).
    ///
    /// Resolves callee FuncId from three call-site shapes:
    /// 1. `Call { func: FuncRef(fid), .. }` — direct top-level function.
    /// 2. `Call { func: Closure { func, captures }, .. }` — inline closure
    ///    expression (rare — usually closures get bound to a name first).
    /// 3. `Call { func: Var(v), .. }` — variable holding a closure/funcref;
    ///    resolved through `var_to_func` (built once per pass from `Bind`
    ///    statements). `param_slot = capture_count + arg_idx`.
    ///
    /// For `MethodCall { obj, method, args, .. }` — resolves the method
    /// FuncId through `resolve_method_func` after `seed_infer`-ing `obj`'s
    /// type. Skips dynamic dispatch where `obj`'s type isn't a known class
    /// (Any/HeapAny/Union/external classes return `None` from the
    /// resolver). For methods, `param_slot = self_offset + arg_idx` where
    /// `self_offset` is 0 for `@staticmethod` and 1 for instance / dunder /
    /// `@classmethod`.
    ///
    /// `visited` is a per-pass cycle-guard: each FuncId is added before
    /// recursing into its body and removed afterwards. Without this, mutual
    /// recursion (`a.m(var)` → calls `b.n(var)` → calls back) loops
    /// forever.
    fn find_elem_via_call_arg(
        &self,
        var: VarId,
        expr: &hir::Expr,
        hir_module: &hir::Module,
        overlay: &IndexMap<VarId, Type>,
        var_to_func: &HashMap<VarId, (FuncId, usize)>,
        visited: &mut HashSet<FuncId>,
    ) -> Option<Type> {
        let mut accum = Type::Never;
        let mut found = false;
        match &expr.kind {
            hir::ExprKind::Call { func, args, .. } => {
                let func_expr = &hir_module.exprs[*func];
                // Resolve callee FuncId from the three supported call-site
                // shapes. `capture_count` is the leading-param offset that
                // must be applied when mapping arg index → callee param slot:
                // captures appear as the first N params of the closure and
                // are not provided by the call site.
                let resolved: Option<(FuncId, usize)> = match &func_expr.kind {
                    hir::ExprKind::FuncRef(fid) => {
                        let cap = self
                            .get_closure_capture_types(fid)
                            .map(|v| v.len())
                            .unwrap_or(0);
                        Some((*fid, cap))
                    }
                    hir::ExprKind::Closure { func, captures } => Some((*func, captures.len())),
                    hir::ExprKind::Var(v) => var_to_func.get(v).copied(),
                    _ => None,
                };
                if let Some((fid, capture_count)) = resolved {
                    if let Some(callee) = hir_module.func_defs.get(&fid) {
                        // Seed callee_overlay with caller-side arg types so
                        // body scans see concrete param types even when the
                        // harvester hasn't yet built `lambda_param_type_hints`
                        // for this callee.
                        let mut callee_overlay = self
                            .lowering_seed_info
                            .per_function_local_seed_types
                            .get(&fid)
                            .cloned()
                            .unwrap_or_default();
                        for (arg_idx, call_arg) in args.iter().enumerate() {
                            let hir::CallArg::Regular(arg_id) = call_arg else {
                                continue;
                            };
                            let arg_expr = &hir_module.exprs[*arg_id];
                            let arg_ty = self.seed_infer_expr_type(arg_expr, hir_module, overlay);
                            if arg_ty == Type::Any || arg_ty == Type::Never {
                                continue;
                            }
                            let param_slot = capture_count + arg_idx;
                            if let Some(param) = callee.params.get(param_slot) {
                                callee_overlay.insert(param.var, arg_ty);
                            }
                        }
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
                            // Cycle-guard before recursing into callee body.
                            if !visited.insert(fid) {
                                continue;
                            }
                            for block in callee.blocks.values() {
                                let result = self.find_elem_type_from_usage(
                                    param.var,
                                    &block.stmts,
                                    hir_module,
                                    &callee_overlay,
                                    var_to_func,
                                    visited,
                                );
                                if result != Type::Any && result != Type::Never {
                                    accum = accum.join(&result);
                                    found = true;
                                }
                            }
                            visited.remove(&fid);
                        }
                    }
                }
                // Recurse into nested call args (e.g. `print(gpt(keys))`).
                for call_arg in args {
                    let hir::CallArg::Regular(arg_id) = call_arg else {
                        continue;
                    };
                    let arg_expr = &hir_module.exprs[*arg_id];
                    if let Some(ty) = self.find_elem_via_call_arg(
                        var,
                        arg_expr,
                        hir_module,
                        overlay,
                        var_to_func,
                        visited,
                    ) {
                        accum = accum.join(&ty);
                        found = true;
                    }
                }
            }
            hir::ExprKind::BuiltinCall { args, .. } => {
                for arg_id in args {
                    let arg_expr = &hir_module.exprs[*arg_id];
                    if let Some(ty) = self.find_elem_via_call_arg(
                        var,
                        arg_expr,
                        hir_module,
                        overlay,
                        var_to_func,
                        visited,
                    ) {
                        accum = accum.join(&ty);
                        found = true;
                    }
                }
            }
            hir::ExprKind::MethodCall {
                obj, method, args, ..
            } => {
                // Recurse into obj/args first — covers `m(...).n(var)`
                // chains and arg-passing patterns. This must happen before
                // method resolution because resolution may fail (Any obj),
                // and we still want to find signals deeper in the tree.
                let obj_expr = &hir_module.exprs[*obj];
                if let Some(ty) = self.find_elem_via_call_arg(
                    var,
                    obj_expr,
                    hir_module,
                    overlay,
                    var_to_func,
                    visited,
                ) {
                    accum = accum.join(&ty);
                    found = true;
                }
                for arg_id in args {
                    let arg_expr = &hir_module.exprs[*arg_id];
                    if let Some(ty) = self.find_elem_via_call_arg(
                        var,
                        arg_expr,
                        hir_module,
                        overlay,
                        var_to_func,
                        visited,
                    ) {
                        accum = accum.join(&ty);
                        found = true;
                    }
                }
                // Resolve `obj.method(...)` to a concrete callee body via
                // class info, then scan that body for `var` mutations.
                // `MethodCall.args: Vec<ExprId>` (no CallArg wrap, unlike
                // `Call.args`), so we iterate ExprIds directly.
                let obj_ty = self.seed_infer_expr_type(obj_expr, hir_module, overlay);
                if let Some(MethodResolution {
                    func_id: fid,
                    self_offset,
                }) = self.resolve_method_func(&obj_ty, *method)
                {
                    if let Some(callee) = hir_module.func_defs.get(&fid) {
                        // Seed callee_overlay with caller-side arg types so
                        // that body usage of OTHER params (`store[idx].append(k)`
                        // — `k` is param 3, not the var we're scanning)
                        // sees the concrete caller-arg type instead of `Any`.
                        // The harvester pass doesn't visit MethodCall arg
                        // types yet, so without this seeding the recursive
                        // `extract_elem_type_from_method_call` call rejects
                        // the appended value as `Any` and the whole chain
                        // collapses to `Never`.
                        let mut callee_overlay = self
                            .lowering_seed_info
                            .per_function_local_seed_types
                            .get(&fid)
                            .cloned()
                            .unwrap_or_default();
                        for (arg_idx, arg_id) in args.iter().enumerate() {
                            let arg_expr = &hir_module.exprs[*arg_id];
                            let arg_ty = self.seed_infer_expr_type(arg_expr, hir_module, overlay);
                            if arg_ty == Type::Any || arg_ty == Type::Never {
                                continue;
                            }
                            let param_slot = self_offset + arg_idx;
                            if let Some(param) = callee.params.get(param_slot) {
                                callee_overlay.insert(param.var, arg_ty);
                            }
                        }
                        for (arg_idx, arg_id) in args.iter().enumerate() {
                            let arg_expr = &hir_module.exprs[*arg_id];
                            if !matches!(&arg_expr.kind, hir::ExprKind::Var(v) if *v == var) {
                                continue;
                            }
                            let param_slot = self_offset + arg_idx;
                            let Some(param) = callee.params.get(param_slot) else {
                                continue;
                            };
                            if !visited.insert(fid) {
                                continue;
                            }
                            for block in callee.blocks.values() {
                                let result = self.find_elem_type_from_usage(
                                    param.var,
                                    &block.stmts,
                                    hir_module,
                                    &callee_overlay,
                                    var_to_func,
                                    visited,
                                );
                                if result != Type::Any && result != Type::Never {
                                    accum = accum.join(&result);
                                    found = true;
                                }
                            }
                            visited.remove(&fid);
                        }
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
