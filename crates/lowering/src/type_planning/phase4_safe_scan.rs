//! Phase 4 (Storage-Uniform) HOF target scan.
//!
//! Walks the HIR module once and records every `FuncId` that flows into a
//! HOF runtime callback slot that cannot accept the Phase 4 tagged user-arg
//! ABI (`sorted`/`min`/`max` key=, `list.sort` key=). These callees are
//! reached via runtime callbacks that deliver elements as raw scalars (legacy
//! ABI) and therefore cannot be flipped to the Phase 4 tagged user-arg ABI.
//!
//! `map`/`filter`/`reduce` callbacks are **not** marked phase4_unsafe:
//! their runtime variants (`rt_map_new_tagged`, `rt_filter_new_tagged`,
//! `rt_reduce_tagged`) deliver elements as tagged Values and handle the
//! return-flipped tagged ABI correctly.
//!
//! The result is stored in `LoweringSeedInfo.phase4_unsafe_funcs`. The
//! lambda-callee prologue extension and closure-tuple `abi_marker`
//! injection consult this set via `is_phase4_safe()` to decide whether a
//! given lambda-like callee uses the tagged or legacy user-arg ABI.
//!
//! ## Variable-stored lambdas (escape analysis)
//!
//! When a lambda is assigned to a variable (`fn_closure = lambda x: ...`),
//! the naive approach marks it phase4_unsafe on the assumption that the
//! address "escapes" through the variable. An escape analysis pre-pass
//! (`build_escaped_lambda_vars`) refines this: if ALL uses of the variable
//! are as HOF callbacks (map/filter/reduce first arg; sorted/min/max key=)
//! the lambda is HOF-only and can stay phase4_safe. Only variables that
//! appear in at least one non-HOF position are considered "escaped" and
//! trigger the unsafe marking.

use std::collections::{HashMap, HashSet};

use pyaot_hir as hir;
use pyaot_utils::{FuncId, StringInterner, VarId};

use crate::context::Lowering;

impl<'a> Lowering<'a> {
    /// Run the HOF target scan over every function body in the module
    /// (including the synthetic module-init function). Idempotent —
    /// re-runs are safe but produce the same set.
    ///
    /// Three-pass:
    /// 1. Build a `Var → FuncId` map from every
    ///    `Bind { target: Var, value: Closure | FuncRef }`.
    /// 2. Escape analysis: compute the set of lambda-bound `Var`s that appear
    ///    in at least one non-HOF position (see `build_escaped_lambda_vars`).
    ///    HOF-only variables (`fn_closure = lambda x: ...; filter(fn_closure,
    ///    ...)`) are NOT marked phase4_unsafe — they stay on the tagged path.
    /// 3. Walk every statement and mark phase4_unsafe only for positions that
    ///    truly take the address (escaped Bind targets, generic Call args, etc.)
    ///    and for HOF callback slots that require legacy raw ABI
    ///    (sorted/min/max key=, list.sort key=).
    pub(crate) fn precompute_phase4_unsafe_funcs(&mut self, hir_module: &hir::Module) {
        let var_to_func = build_phase4_var_to_func(hir_module);
        let escaped_vars = build_escaped_lambda_vars(hir_module, &var_to_func, self.interner);
        for func_id in &hir_module.functions {
            if let Some(func) = hir_module.func_defs.get(func_id) {
                for block in func.blocks.values() {
                    for stmt_id in &block.stmts {
                        let stmt = &hir_module.stmts[*stmt_id];
                        self.scan_stmt_for_phase4_unsafe(
                            stmt,
                            hir_module,
                            &var_to_func,
                            &escaped_vars,
                        );
                    }
                    // Block terminators carry expressions too: `return`,
                    // `raise`, branch conditions and `yield` are
                    // `HirTerminator` variants, not `block.stmts`. Without
                    // this a HOF callback in terminator position (e.g.
                    // `return sorted(xs, key=lambda ...)`) would never be
                    // scanned. Mirrors `build_escaped_lambda_vars`.
                    self.scan_terminator_for_phase4_unsafe(
                        &block.terminator,
                        hir_module,
                        &var_to_func,
                    );
                }
            }
        }
    }

    fn scan_stmt_for_phase4_unsafe(
        &mut self,
        stmt: &hir::Stmt,
        hir_module: &hir::Module,
        var_to_func: &HashMap<VarId, Vec<FuncId>>,
        escaped_vars: &HashSet<VarId>,
    ) {
        match &stmt.kind {
            hir::StmtKind::Bind { value, target, .. } => {
                let expr = &hir_module.exprs[*value];
                // Bind value is an "address-taken-equivalent" position:
                // `f = lambda ...` and `f = decorator(orig)` both bind a
                // callable to a Var. The lambda's address (or any
                // FuncRef inside the value) escapes through the Var.
                //
                // Escape-analysis refinement: if the binding target is a
                // simple Var, only mark the FuncRef as phase4_unsafe when
                // the escape analysis determined the variable appears in at
                // least one non-HOF position. If ALL uses are HOF callbacks
                // (map/filter/reduce first arg; sorted/min/max key=), the
                // lambda can stay phase4_safe and take the tagged ABI path.
                //
                // Class-attribute targets are NOT skipped: `Cls.handler =
                // lambda ...` stores a genuine first-class callable into a
                // class slot from which it can be fetched and called with
                // an unknown ABI. Plain method definitions also flow here,
                // but marking a method phase4_unsafe is harmless — methods
                // are already excluded from return-ABI flipping.
                let var_escapes = match target {
                    hir::BindingTarget::Var(vid) => escaped_vars.contains(vid),
                    // Non-Var binding targets (attr, index, tuple, class
                    // attr) always count as escaped — the lambda flows into
                    // a container / object / class slot where it may be
                    // called with an unknown ABI.
                    _ => true,
                };
                if var_escapes {
                    self.mark_address_taken_funcrefs(expr, hir_module);
                }
                self.scan_expr_for_phase4_unsafe(expr, hir_module, var_to_func);
            }
            hir::StmtKind::Expr(expr_id) => {
                let expr = &hir_module.exprs[*expr_id];
                self.scan_expr_for_phase4_unsafe(expr, hir_module, var_to_func);
            }
            hir::StmtKind::Return(Some(expr_id)) => {
                let expr = &hir_module.exprs[*expr_id];
                // Return position is NOT treated as address-taking. A
                // returned closure / FuncRef reaches its consumer only
                // through the closure trampoline
                // `rt_call_with_captures_and_args`, which dispatches on
                // the marker bit and propagates the callee's tagged
                // return verbatim. Caller-side `emit_closure_call`
                // unwraps the tagged Value into a primitive dest when
                // the marker bit is set, so a phase4-safe inner
                // function returned from an outer factory (decorator-
                // factory / curried-chain pattern) flows correctly
                // end-to-end. The earlier conservative marking caused
                // every returned inner func to be `phase4_unsafe`,
                // blocking the lambda return-flip from reaching the
                // chain-style code that needed it most.
                self.scan_expr_for_phase4_unsafe(expr, hir_module, var_to_func);
            }
            hir::StmtKind::Assert { cond, msg } => {
                let expr = &hir_module.exprs[*cond];
                self.scan_expr_for_phase4_unsafe(expr, hir_module, var_to_func);
                if let Some(msg_id) = msg {
                    let msg_expr = &hir_module.exprs[*msg_id];
                    self.scan_expr_for_phase4_unsafe(msg_expr, hir_module, var_to_func);
                }
            }
            hir::StmtKind::IterSetup { iter } => {
                let expr = &hir_module.exprs[*iter];
                self.scan_expr_for_phase4_unsafe(expr, hir_module, var_to_func);
            }
            hir::StmtKind::IterAdvance { iter, .. } => {
                let expr = &hir_module.exprs[*iter];
                self.scan_expr_for_phase4_unsafe(expr, hir_module, var_to_func);
            }
            hir::StmtKind::IndexDelete { obj, index } => {
                self.scan_expr_for_phase4_unsafe(&hir_module.exprs[*obj], hir_module, var_to_func);
                self.scan_expr_for_phase4_unsafe(
                    &hir_module.exprs[*index],
                    hir_module,
                    var_to_func,
                );
            }
            hir::StmtKind::Raise { exc, cause } => {
                if let Some(e) = exc {
                    self.scan_expr_for_phase4_unsafe(
                        &hir_module.exprs[*e],
                        hir_module,
                        var_to_func,
                    );
                }
                if let Some(c) = cause {
                    self.scan_expr_for_phase4_unsafe(
                        &hir_module.exprs[*c],
                        hir_module,
                        var_to_func,
                    );
                }
            }
            _ => {}
        }
    }

    /// Scan a block terminator for HOF callbacks and address-taken
    /// FuncRefs. `return` / `raise` / branch conditions / `yield` are
    /// `HirTerminator` variants (never pushed into `block.stmts`), so the
    /// `StmtKind::Return` / `StmtKind::Raise` arms of
    /// `scan_stmt_for_phase4_unsafe` never fire — this is the path that
    /// actually reaches terminator-position expressions.
    fn scan_terminator_for_phase4_unsafe(
        &mut self,
        terminator: &hir::HirTerminator,
        hir_module: &hir::Module,
        var_to_func: &HashMap<VarId, Vec<FuncId>>,
    ) {
        match terminator {
            hir::HirTerminator::Return(Some(expr_id)) => {
                // Return is NOT address-taking (see the `StmtKind::Return`
                // rationale above) — only scan for nested HOF callbacks.
                self.scan_expr_for_phase4_unsafe(
                    &hir_module.exprs[*expr_id],
                    hir_module,
                    var_to_func,
                );
            }
            hir::HirTerminator::Branch { cond, .. } => {
                self.scan_expr_for_phase4_unsafe(&hir_module.exprs[*cond], hir_module, var_to_func);
            }
            hir::HirTerminator::Raise { exc, cause } => {
                self.scan_expr_for_phase4_unsafe(&hir_module.exprs[*exc], hir_module, var_to_func);
                if let Some(c) = cause {
                    self.scan_expr_for_phase4_unsafe(
                        &hir_module.exprs[*c],
                        hir_module,
                        var_to_func,
                    );
                }
            }
            hir::HirTerminator::Yield { value, .. } => {
                self.scan_expr_for_phase4_unsafe(
                    &hir_module.exprs[*value],
                    hir_module,
                    var_to_func,
                );
            }
            hir::HirTerminator::Return(None)
            | hir::HirTerminator::Jump(_)
            | hir::HirTerminator::Reraise
            | hir::HirTerminator::Unreachable => {}
        }
    }

    fn scan_expr_for_phase4_unsafe(
        &mut self,
        expr: &hir::Expr,
        hir_module: &hir::Module,
        var_to_func: &HashMap<VarId, Vec<FuncId>>,
    ) {
        match &expr.kind {
            // Phase 4+ Extension E2d: map/filter/reduce callbacks are no
            // longer marked phase4_unsafe (tagged-delivery variants exist).
            // sorted/min/max key= still need the marker — the key callback
            // returns raw scalars; no tagged-value variant exists yet.
            hir::ExprKind::BuiltinCall {
                builtin, kwargs, ..
            } => {
                if matches!(
                    builtin,
                    hir::Builtin::Sorted | hir::Builtin::Min | hir::Builtin::Max
                ) {
                    for kw in kwargs {
                        if self.interner.resolve(kw.name) == "key" {
                            self.mark_callback_phase4_unsafe(
                                &hir_module.exprs[kw.value],
                                hir_module,
                                var_to_func,
                            );
                        }
                    }
                }
                // Fall through: for_each_subexpr_id recurses args + kwargs.
            }
            // list.sort(key=…) — record the key callback as HOF-targeted.
            // Args and kwargs are address-taken positions (mirrors Call below).
            hir::ExprKind::MethodCall {
                method,
                args,
                kwargs,
                ..
            } => {
                if self.interner.resolve(*method) == "sort" {
                    for kw in kwargs {
                        if self.interner.resolve(kw.name) == "key" {
                            self.mark_callback_phase4_unsafe(
                                &hir_module.exprs[kw.value],
                                hir_module,
                                var_to_func,
                            );
                        }
                    }
                }
                for arg_id in args {
                    self.mark_address_taken_funcrefs(&hir_module.exprs[*arg_id], hir_module);
                }
                for kw in kwargs {
                    self.mark_address_taken_funcrefs(&hir_module.exprs[kw.value], hir_module);
                }
                // Fall through: for_each_subexpr_id recurses obj + args + kwargs.
            }
            // Call.func: direct-call position, not address-taken — carve-out.
            // Call.args / kwargs / kwargs_unpack: FuncRef here is address-taken
            // (decorator factory, callback, etc.).
            hir::ExprKind::Call {
                args,
                kwargs,
                kwargs_unpack,
                ..
            } => {
                for arg in args {
                    let arg_id = match arg {
                        hir::CallArg::Regular(id) | hir::CallArg::Starred(id) => id,
                    };
                    self.mark_address_taken_funcrefs(&hir_module.exprs[*arg_id], hir_module);
                }
                for kw in kwargs {
                    self.mark_address_taken_funcrefs(&hir_module.exprs[kw.value], hir_module);
                }
                if let Some(unpack_id) = kwargs_unpack {
                    self.mark_address_taken_funcrefs(&hir_module.exprs[*unpack_id], hir_module);
                }
                // Fall through: for_each_subexpr_id recurses func + args + kwargs +
                // kwargs_unpack via scan_expr_for_phase4_unsafe.
            }
            // `super().method(args)` — args are address-taken positions.
            hir::ExprKind::SuperCall { args, .. } => {
                for arg_id in args {
                    self.mark_address_taken_funcrefs(&hir_module.exprs[*arg_id], hir_module);
                }
                // Fall through: for_each_subexpr_id recurses args.
            }
            // Stdlib call — same as SuperCall.
            hir::ExprKind::StdlibCall { args, .. } => {
                for arg_id in args {
                    self.mark_address_taken_funcrefs(&hir_module.exprs[*arg_id], hir_module);
                }
                // Fall through: for_each_subexpr_id recurses args.
            }
            _ => {}
        }
        // Default structural recursion via exhaustive helper. Routing through
        // `for_each_subexpr_id` means a new `ExprKind` variant is a compile
        // error there — every scanner that uses this helper inherits the fix.
        //
        // Borrow-checker: collect sub-expression ids first, then recurse.
        let mut sub_ids: smallvec::SmallVec<[hir::ExprId; 4]> = smallvec::SmallVec::new();
        hir::visit::for_each_subexpr_id(expr, hir_module, |id| sub_ids.push(id));
        for id in sub_ids {
            self.scan_expr_for_phase4_unsafe(&hir_module.exprs[id], hir_module, var_to_func);
        }
    }

    /// Mark every `FuncRef` that appears anywhere in `expr` as
    /// phase4_unsafe. Use this on expressions that are syntactically
    /// "values" (Call.args, kwargs, Closure.captures, container literals,
    /// Return value, etc.) — a `FuncRef` here means the
    /// function's address is being passed as a callable value, which may
    /// flow into a runtime-erased indirect call site that abi_repair
    /// cannot coerce. `Closure { func, captures }` similarly captures the
    /// inner func by address; mark its `func` too.
    ///
    /// Recursion mirrors `scan_expr_for_phase4_unsafe`'s structural walk,
    /// minus the Call.func position carve-out — every position visited
    /// here treats FuncRef as address-taken.
    fn mark_address_taken_funcrefs(&mut self, expr: &hir::Expr, hir_module: &hir::Module) {
        match &expr.kind {
            // Direct FuncRef — the function's address is being taken.
            hir::ExprKind::FuncRef(func_id) => {
                self.lowering_seed_info.phase4_unsafe_funcs.insert(*func_id);
                // Leaf — for_each_subexpr_id visits nothing.
            }
            // Closure — inner func is also address-taken; captures recurse below.
            hir::ExprKind::Closure { func, .. } => {
                self.lowering_seed_info.phase4_unsafe_funcs.insert(*func);
                // Fall through: for_each_subexpr_id recurses into captures.
            }
            _ => {}
        }
        // Default structural recursion via exhaustive helper.
        //
        // Borrow-checker: collect sub-expression ids first, then recurse.
        let mut sub_ids: smallvec::SmallVec<[hir::ExprId; 4]> = smallvec::SmallVec::new();
        hir::visit::for_each_subexpr_id(expr, hir_module, |id| sub_ids.push(id));
        for id in sub_ids {
            self.mark_address_taken_funcrefs(&hir_module.exprs[id], hir_module);
        }
    }

    fn mark_callback_phase4_unsafe(
        &mut self,
        expr: &hir::Expr,
        hir_module: &hir::Module,
        var_to_func: &HashMap<VarId, Vec<FuncId>>,
    ) {
        match &expr.kind {
            hir::ExprKind::FuncRef(func_id) => {
                self.lowering_seed_info.phase4_unsafe_funcs.insert(*func_id);
            }
            hir::ExprKind::Closure { func, captures } => {
                self.lowering_seed_info.phase4_unsafe_funcs.insert(*func);
                // A captured FuncRef / Closure is address-taken — mark it
                // unsafe. `scan_expr_for_phase4_unsafe` has no FuncRef arm,
                // so routing captures through it would leave a captured
                // callback (e.g. `key=lambda x: helper(x)` with `helper`
                // captured) wrongly phase4_safe.
                for cap_id in captures {
                    self.mark_address_taken_funcrefs(&hir_module.exprs[*cap_id], hir_module);
                }
            }
            hir::ExprKind::Var(var_id) => {
                // Indirect callback: `f = lambda ...; sorted(xs, key=f)`.
                // `var_to_func` records EVERY function bound to the var
                // (a var may be rebound — `f = key_a; ...; f = key_b`), so
                // mark them all: we cannot statically tell which binding is
                // live at this call site, and a missed one would be wrongly
                // return-ABI-flipped while reached via a raw-scalar HOF.
                //
                // TODO(#7): a var bound to a `Call` result (`f = factory()`)
                // is not recorded in `var_to_func` and stays phase4_safe —
                // resolving it needs interprocedural analysis.
                if let Some(func_ids) = var_to_func.get(var_id) {
                    for func_id in func_ids {
                        self.lowering_seed_info.phase4_unsafe_funcs.insert(*func_id);
                    }
                }
            }
            _ => {}
        }
    }

    /// Returns true when `func_id` may be flipped to the Phase 4 tagged
    /// user-arg ABI: not reached via any HOF runtime callback that delivers
    /// elements as raw scalars (sorted/min/max key=, list.sort key=).
    /// map/filter/reduce callbacks are phase4-safe: their runtime variants
    /// deliver elements as tagged Values and accept the flipped return ABI.
    /// Consulted by `function_lowering` for user-param and return-ABI flip
    /// eligibility, by HOF lowering for `rt_map/reduce/filter_tagged` routing,
    /// and by closure-tuple marker-bit injection.
    pub(crate) fn is_phase4_safe(&self, func_id: pyaot_utils::FuncId) -> bool {
        !self
            .lowering_seed_info
            .phase4_unsafe_funcs
            .contains(&func_id)
    }

    /// Phase 4+ Extension E2d: mirror of the `phase4_return_abi_flipped`
    /// eligibility predicate from `function_lowering::lower_function`,
    /// usable from sibling lowering paths (HOF runtime call lowering)
    /// that need to know whether the callee will be return-flipped
    /// without instantiating the full lowering pipeline. Used by
    /// `lower_map` / `lower_filter` / `lower_reduce` to decide whether
    /// to encode `result_box_kind = 0` (pass-through, callee already
    /// boxes its return) vs `1`/`2` (legacy re-wrap, callee returns raw
    /// primitive bits — i.e. lambdas).
    ///
    /// Returns true iff: not a class method, not a generator resume, not
    /// module init, has an explicit primitive return annotation, and
    /// `is_phase4_safe`. Lambdas, nested functions, and genexp creators
    /// are eligible — their bodies box the return operand and the
    /// closure trampoline / direct dispatch path propagates the tagged
    /// Value to the caller.
    pub(crate) fn is_return_abi_flipped(
        &self,
        func_id: pyaot_utils::FuncId,
        hir_module: &pyaot_hir::Module,
    ) -> bool {
        let Some(func) = hir_module.func_defs.get(&func_id) else {
            return false;
        };
        let name = self.interner.resolve(func.name);
        let is_module_init = name == "__pyaot_module_init__";
        let is_generator_resume = func_id.0 >= pyaot_utils::RESUME_FUNC_ID_OFFSET;
        if is_module_init || is_generator_resume {
            return false;
        }
        // Class methods are excluded from return-ABI flipping — see
        // `function_lowering.rs` for the rationale. The
        // `flippable_method_funcs` override was removed in Stage E.4.
        // @property getter/setter functions live in ClassDef::properties
        // (not ClassDef::methods), so they are explicitly included here;
        // see Stage E.3 follow-up in function_lowering.rs for context.
        let is_class_method = hir_module.class_defs.values().any(|cd| {
            cd.methods.contains(&func_id)
                || cd.init_method == Some(func_id)
                || cd
                    .properties
                    .iter()
                    .any(|p| p.getter == func_id || p.setter == Some(func_id))
        });
        if is_class_method {
            return false;
        }
        let has_explicit_return_type = func.return_type.is_some()
            && func.return_type.as_ref() != Some(&pyaot_types::Type::None);
        if !has_explicit_return_type {
            return false;
        }
        let return_primitive_typed = matches!(
            func.return_type,
            Some(pyaot_types::Type::Int)
                | Some(pyaot_types::Type::Bool)
                | Some(pyaot_types::Type::Float)
        );
        return_primitive_typed && self.is_phase4_safe(func_id)
    }
}

/// Build a `Var → [FuncId]` resolution map by scanning every
/// `Bind { target: Var, value: Closure | FuncRef | IfExpr<...> }` in the
/// module. Mirrors the `var_to_func` pattern in
/// `closure_scan::infer_nested_function_param_types_inner`.
///
/// A var reassigned across multiple binds (`f = key_a; ...; f = key_b`)
/// accumulates ALL bound functions — recording only the last binding
/// would leave the other lambda wrongly phase4_safe even though it may
/// be the one that actually reaches a raw-scalar HOF callback slot.
fn build_phase4_var_to_func(hir_module: &hir::Module) -> HashMap<VarId, Vec<FuncId>> {
    let mut map: HashMap<VarId, Vec<FuncId>> = HashMap::new();
    for (_stmt_id, stmt) in hir_module.stmts.iter() {
        let hir::StmtKind::Bind { target, value, .. } = &stmt.kind else {
            continue;
        };
        let hir::BindingTarget::Var(var_id) = target else {
            continue;
        };
        let mut funcs: Vec<FuncId> = Vec::new();
        collect_phase4_bound_funcs(&hir_module.exprs[*value], hir_module, &mut funcs);
        if !funcs.is_empty() {
            let entry = map.entry(*var_id).or_default();
            // Skip duplicates — same FuncId appears when an IfExpr has
            // matching branches (`f = lam if c else lam`) or when a var is
            // rebound to the same lambda. Downstream consumers iterate the
            // vec; HashSet::insert would dedupe but reading the vec twice
            // is wasted work.
            for f in funcs {
                if !entry.contains(&f) {
                    entry.push(f);
                }
            }
        }
    }
    map
}

/// Collect every `FuncId` a binding-value expression may statically
/// resolve to: `Closure` / `FuncRef` literals, recursing through both
/// branches of an `IfExpr` (`f = a_lambda if cond else b_lambda`). A
/// `Call` result is not statically resolvable here and is intentionally
/// left out — see the `TODO(#7)` in `mark_callback_phase4_unsafe`.
fn collect_phase4_bound_funcs(expr: &hir::Expr, hir_module: &hir::Module, out: &mut Vec<FuncId>) {
    match &expr.kind {
        hir::ExprKind::Closure { func, .. } => out.push(*func),
        hir::ExprKind::FuncRef(func_id) => out.push(*func_id),
        hir::ExprKind::IfExpr {
            then_val, else_val, ..
        } => {
            collect_phase4_bound_funcs(&hir_module.exprs[*then_val], hir_module, out);
            collect_phase4_bound_funcs(&hir_module.exprs[*else_val], hir_module, out);
        }
        _ => {}
    }
}

/// Escape analysis for lambda-bound variables.
///
/// For each `VarId` in `var_to_func` (i.e., a variable holding a lambda or
/// named function), determine whether that variable appears in at least one
/// position that is NOT a tagged HOF callback slot. If so, the variable
/// "escapes" and its lambda must be marked phase4_unsafe.
///
/// "HOF callback slots" that are safe (tagged-delivery variants exist):
/// - First argument of `map(fn, iterable)` — `rt_map_new_tagged`
/// - First argument of `filter(fn, iterable)` — `rt_filter_new_tagged`
/// - First argument of `reduce(fn, iterable)` — `rt_reduce_tagged`
///
/// "HOF callback slots" that are unsafe (raw-scalar delivery, no tagged variant):
/// - `key=` kwarg of `sorted(...)`, `min(...)`, `max(...)` — still raw
/// - `key=` kwarg of `list.sort(...)` — still raw
///
/// Any other use of a lambda-bound variable — including being passed to a
/// generic `Call`, stored in a container, returned, etc. — is treated as
/// escaping.
///
/// Returns a `HashSet<VarId>` of variables that escaped to a non-HOF-safe
/// position. Variables NOT in the set are HOF-only and can stay phase4_safe.
fn build_escaped_lambda_vars(
    hir_module: &hir::Module,
    var_to_func: &HashMap<VarId, Vec<FuncId>>,
    interner: &StringInterner,
) -> HashSet<VarId> {
    let mut escaped: HashSet<VarId> = HashSet::new();

    // Walk every expression in the module — both ordinary statements AND
    // block terminators. Important: the HIR CFG builder converts
    // `StmtKind::Return` / `StmtKind::Raise` into `HirTerminator::Return`
    // / `HirTerminator::Raise` (not pushed into `block.stmts`). So we
    // must scan terminators separately to catch `return fn_var` patterns.
    for func_id in &hir_module.functions {
        let Some(func) = hir_module.func_defs.get(func_id) else {
            continue;
        };
        for block in func.blocks.values() {
            for stmt_id in &block.stmts {
                let stmt = &hir_module.stmts[*stmt_id];
                scan_stmt_for_escaped_lambda_vars(
                    stmt,
                    hir_module,
                    var_to_func,
                    interner,
                    &mut escaped,
                );
            }
            // Also scan the block terminator for lambda-var escapes.
            scan_terminator_for_escaped_lambda_vars(
                &block.terminator,
                hir_module,
                var_to_func,
                interner,
                &mut escaped,
            );
        }
    }
    escaped
}

/// Scan a block terminator for lambda-var uses that escape to non-HOF positions.
///
/// `HirTerminator::Return` is the primary concern: `return fn_var` makes the
/// lambda-bound variable escape through the return value. Decorator wrappers
/// returned from factory functions must be marked phase4_unsafe so they are
/// not return-ABI-flipped (their call sites expect Raw(I64), not Tagged).
fn scan_terminator_for_escaped_lambda_vars(
    terminator: &hir::HirTerminator,
    hir_module: &hir::Module,
    var_to_func: &HashMap<VarId, Vec<FuncId>>,
    interner: &StringInterner,
    escaped: &mut HashSet<VarId>,
) {
    match terminator {
        hir::HirTerminator::Return(Some(expr_id)) => {
            // Return: conservatively treat as an escape. See the rationale in
            // `scan_stmt_for_escaped_lambda_vars` (Return case).
            mark_escaped_in_expr_id(*expr_id, hir_module, var_to_func, interner, escaped);
        }
        hir::HirTerminator::Branch { cond, .. } => {
            mark_escaped_in_expr_id(*cond, hir_module, var_to_func, interner, escaped);
        }
        hir::HirTerminator::Raise { exc, cause } => {
            mark_escaped_in_expr_id(*exc, hir_module, var_to_func, interner, escaped);
            if let Some(c) = cause {
                mark_escaped_in_expr_id(*c, hir_module, var_to_func, interner, escaped);
            }
        }
        hir::HirTerminator::Yield { value, .. } => {
            mark_escaped_in_expr_id(*value, hir_module, var_to_func, interner, escaped);
        }
        _ => {}
    }
}

/// Scan a single statement for lambda-var uses that escape to non-HOF positions.
fn scan_stmt_for_escaped_lambda_vars(
    stmt: &hir::Stmt,
    hir_module: &hir::Module,
    var_to_func: &HashMap<VarId, Vec<FuncId>>,
    interner: &StringInterner,
    escaped: &mut HashSet<VarId>,
) {
    match &stmt.kind {
        hir::StmtKind::Bind { value, target, .. } => {
            // When the target is a simple Var and the value is a Closure/FuncRef,
            // this is the definition site — skip it; we track uses separately.
            // For all other bind targets (attr, index, tuple destructuring), or
            // when the value is a Var that resolves to a lambda, the lambda escapes.
            let value_expr = &hir_module.exprs[*value];
            let is_lambda_definition = matches!(
                value_expr.kind,
                hir::ExprKind::Closure { .. } | hir::ExprKind::FuncRef(_)
            ) && matches!(target, hir::BindingTarget::Var(_));
            if !is_lambda_definition {
                mark_escaped_in_expr(value_expr, hir_module, var_to_func, interner, escaped);
            }
            // Also scan sub-expressions of the target (attr objects, index objs).
            scan_binding_target_for_escaped(target, hir_module, var_to_func, interner, escaped);
        }
        hir::StmtKind::Expr(expr_id) => {
            mark_escaped_in_expr_id(*expr_id, hir_module, var_to_func, interner, escaped);
        }
        hir::StmtKind::Return(Some(expr_id)) => {
            // Return position: conservatively treat as an escape. A decorator
            // wrapper returned from a factory (`return wrapper`) gets assigned
            // to the decorated name and called DIRECTLY (CallDirect with a Raw
            // dest), not via the closure trampoline. If we omit the escape mark
            // the wrapper gets return-ABI-flipped (→ Tagged return) while the
            // call site still expects Raw(I64) → verifier violation.
            //
            // The closure-trampoline path (marker-bit dispatch) is used only
            // for HOF callbacks stored in Closures, not for named functions
            // returned from decorators. So Return(Var) must be treated as
            // escaping to preserve the Raw return ABI for wrapper functions.
            mark_escaped_in_expr_id(*expr_id, hir_module, var_to_func, interner, escaped);
        }
        hir::StmtKind::Assert { cond, msg } => {
            mark_escaped_in_expr_id(*cond, hir_module, var_to_func, interner, escaped);
            if let Some(msg_id) = msg {
                mark_escaped_in_expr_id(*msg_id, hir_module, var_to_func, interner, escaped);
            }
        }
        hir::StmtKind::IterSetup { iter } => {
            mark_escaped_in_expr_id(*iter, hir_module, var_to_func, interner, escaped);
        }
        hir::StmtKind::IterAdvance { iter, .. } => {
            mark_escaped_in_expr_id(*iter, hir_module, var_to_func, interner, escaped);
        }
        hir::StmtKind::IndexDelete { obj, index } => {
            mark_escaped_in_expr_id(*obj, hir_module, var_to_func, interner, escaped);
            mark_escaped_in_expr_id(*index, hir_module, var_to_func, interner, escaped);
        }
        hir::StmtKind::Raise { exc, cause } => {
            if let Some(e) = exc {
                mark_escaped_in_expr_id(*e, hir_module, var_to_func, interner, escaped);
            }
            if let Some(c) = cause {
                mark_escaped_in_expr_id(*c, hir_module, var_to_func, interner, escaped);
            }
        }
        _ => {}
    }
}

/// Helper: scan a BindingTarget for any lambda-var uses that could escape.
fn scan_binding_target_for_escaped(
    target: &hir::BindingTarget,
    hir_module: &hir::Module,
    var_to_func: &HashMap<VarId, Vec<FuncId>>,
    interner: &StringInterner,
    escaped: &mut HashSet<VarId>,
) {
    match target {
        hir::BindingTarget::Attr { obj, .. } => {
            mark_escaped_in_expr_id(*obj, hir_module, var_to_func, interner, escaped);
        }
        hir::BindingTarget::Index { obj, index, .. } => {
            mark_escaped_in_expr_id(*obj, hir_module, var_to_func, interner, escaped);
            mark_escaped_in_expr_id(*index, hir_module, var_to_func, interner, escaped);
        }
        hir::BindingTarget::Tuple { elts, .. } => {
            for elt in elts {
                scan_binding_target_for_escaped(elt, hir_module, var_to_func, interner, escaped);
            }
        }
        _ => {}
    }
}

/// Mark any lambda-bound `Var` that appears directly or transitively in
/// `expr` as escaped — UNLESS it appears in a known tagged-HOF callback slot
/// (map/filter/reduce first arg).
///
/// HOF-safe positions are carved out explicitly: the first argument of
/// `Builtin::Map`, `Builtin::Filter`, and `Builtin::Reduce` is skipped (not
/// recursed into for escape marking). All other positions call this function
/// recursively.
fn mark_escaped_in_expr(
    expr: &hir::Expr,
    hir_module: &hir::Module,
    var_to_func: &HashMap<VarId, Vec<FuncId>>,
    interner: &StringInterner,
    escaped: &mut HashSet<VarId>,
) {
    match &expr.kind {
        hir::ExprKind::Var(var_id) => {
            // Any Var in a non-HOF position that holds a lambda → escaped.
            if var_to_func.contains_key(var_id) {
                escaped.insert(*var_id);
            }
        }
        hir::ExprKind::BuiltinCall {
            builtin,
            args,
            kwargs,
            ..
        } => {
            // map/filter/reduce first arg is a HOF callback slot — skip it for
            // escape marking. All other args/kwargs are regular value positions.
            let skip_first_arg = matches!(
                builtin,
                hir::Builtin::Map | hir::Builtin::Filter | hir::Builtin::Reduce
            );
            for (i, arg_id) in args.iter().enumerate() {
                if skip_first_arg && i == 0 {
                    // HOF callback slot: not an escape for map/filter/reduce.
                    // Still recurse into sub-expressions of the arg (e.g., a
                    // Closure's capture list), but a bare Var here is safe.
                    mark_escaped_in_expr_hof_arg(
                        &hir_module.exprs[*arg_id],
                        hir_module,
                        var_to_func,
                        interner,
                        escaped,
                    );
                } else {
                    mark_escaped_in_expr_id(*arg_id, hir_module, var_to_func, interner, escaped);
                }
            }
            // kwargs: sorted/min/max key= is an unsafe HOF slot, handled in
            // the unsafe-marking pass. For the escape-analysis pre-pass, treat
            // key= as an escape position (it will be marked unsafe by
            // mark_callback_phase4_unsafe in the main scan).
            for kw in kwargs {
                mark_escaped_in_expr_id(kw.value, hir_module, var_to_func, interner, escaped);
            }
        }
        hir::ExprKind::MethodCall {
            obj,
            method,
            args,
            kwargs,
        } => {
            mark_escaped_in_expr_id(*obj, hir_module, var_to_func, interner, escaped);
            // list.sort(key=) is an unsafe HOF slot — mark key= Var as escaped.
            let is_sort = interner.resolve(*method) == "sort";
            for arg_id in args {
                mark_escaped_in_expr_id(*arg_id, hir_module, var_to_func, interner, escaped);
            }
            for kw in kwargs {
                if is_sort && interner.resolve(kw.name) == "key" {
                    // key= of sort is an unsafe HOF slot — treat as escape so
                    // the lambda gets marked phase4_unsafe by the main scan.
                    mark_escaped_in_expr_id(kw.value, hir_module, var_to_func, interner, escaped);
                } else {
                    mark_escaped_in_expr_id(kw.value, hir_module, var_to_func, interner, escaped);
                }
            }
        }
        hir::ExprKind::Call {
            func, args, kwargs, ..
        } => {
            // Call.func is a direct call — not an escape of the Var.
            // Call.args/kwargs: passing a lambda-Var as an argument to a
            // generic call (not a known HOF) is an escape.
            mark_escaped_in_expr_id(*func, hir_module, var_to_func, interner, escaped);
            for arg in args {
                let arg_id = match arg {
                    hir::CallArg::Regular(id) | hir::CallArg::Starred(id) => *id,
                };
                mark_escaped_in_expr_id(arg_id, hir_module, var_to_func, interner, escaped);
            }
            for kw in kwargs {
                mark_escaped_in_expr_id(kw.value, hir_module, var_to_func, interner, escaped);
            }
        }
        hir::ExprKind::Closure { captures, .. } => {
            for cap_id in captures {
                mark_escaped_in_expr_id(*cap_id, hir_module, var_to_func, interner, escaped);
            }
        }
        hir::ExprKind::List(items) | hir::ExprKind::Tuple(items) | hir::ExprKind::Set(items) => {
            for item in items {
                mark_escaped_in_expr_id(*item, hir_module, var_to_func, interner, escaped);
            }
        }
        hir::ExprKind::Dict(pairs) => {
            for (k, v) in pairs {
                mark_escaped_in_expr_id(*k, hir_module, var_to_func, interner, escaped);
                mark_escaped_in_expr_id(*v, hir_module, var_to_func, interner, escaped);
            }
        }
        hir::ExprKind::BinOp { left, right, .. }
        | hir::ExprKind::Compare { left, right, .. }
        | hir::ExprKind::LogicalOp { left, right, .. } => {
            mark_escaped_in_expr_id(*left, hir_module, var_to_func, interner, escaped);
            mark_escaped_in_expr_id(*right, hir_module, var_to_func, interner, escaped);
        }
        hir::ExprKind::UnOp { operand, .. } => {
            mark_escaped_in_expr_id(*operand, hir_module, var_to_func, interner, escaped);
        }
        hir::ExprKind::Attribute { obj, .. } => {
            mark_escaped_in_expr_id(*obj, hir_module, var_to_func, interner, escaped);
        }
        hir::ExprKind::Index { obj, index } => {
            mark_escaped_in_expr_id(*obj, hir_module, var_to_func, interner, escaped);
            mark_escaped_in_expr_id(*index, hir_module, var_to_func, interner, escaped);
        }
        hir::ExprKind::IfExpr {
            cond,
            then_val,
            else_val,
        } => {
            mark_escaped_in_expr_id(*cond, hir_module, var_to_func, interner, escaped);
            mark_escaped_in_expr_id(*then_val, hir_module, var_to_func, interner, escaped);
            mark_escaped_in_expr_id(*else_val, hir_module, var_to_func, interner, escaped);
        }
        // Default recursion via the exhaustive `for_each_subexpr_id`
        // helper — Slice / SuperCall / StdlibCall / FormatSpec / Yield /
        // IterHasNext / MatchPattern / leaves all route here. Previously
        // `_ => {}` silently dropped them. Adding a new `ExprKind`
        // variant now produces a compile error in
        // `hir::visit::for_each_subexpr_id` instead of a silent miss.
        _ => {
            let mut sub_ids: smallvec::SmallVec<[hir::ExprId; 4]> = smallvec::SmallVec::new();
            hir::visit::for_each_subexpr_id(expr, hir_module, |id| sub_ids.push(id));
            for id in sub_ids {
                mark_escaped_in_expr_id(id, hir_module, var_to_func, interner, escaped);
            }
        }
    }
}

/// Scan an ExprId for escaped lambda vars.
fn mark_escaped_in_expr_id(
    expr_id: hir::ExprId,
    hir_module: &hir::Module,
    var_to_func: &HashMap<VarId, Vec<FuncId>>,
    interner: &StringInterner,
    escaped: &mut HashSet<VarId>,
) {
    mark_escaped_in_expr(
        &hir_module.exprs[expr_id],
        hir_module,
        var_to_func,
        interner,
        escaped,
    );
}

/// Scan a HOF-callback-slot argument for escape. A bare `Var` here is safe
/// (the variable flows directly into a tagged HOF slot). Sub-expressions
/// (capture lists of inline closures, etc.) are still scanned normally.
fn mark_escaped_in_expr_hof_arg(
    expr: &hir::Expr,
    hir_module: &hir::Module,
    var_to_func: &HashMap<VarId, Vec<FuncId>>,
    interner: &StringInterner,
    escaped: &mut HashSet<VarId>,
) {
    match &expr.kind {
        // Bare Var in HOF arg position — NOT an escape.
        hir::ExprKind::Var(_) => {}
        // Inline Closure in HOF arg position — also not an escape.
        // Still recurse into captures in case they reference lambda-bound Vars.
        hir::ExprKind::Closure { captures, .. } => {
            for cap_id in captures {
                mark_escaped_in_expr_id(*cap_id, hir_module, var_to_func, interner, escaped);
            }
        }
        // FuncRef directly inline — not an escape.
        hir::ExprKind::FuncRef(_) => {}
        // Anything else: fall back to the regular escape scan.
        _ => {
            mark_escaped_in_expr(expr, hir_module, var_to_func, interner, escaped);
        }
    }
}
