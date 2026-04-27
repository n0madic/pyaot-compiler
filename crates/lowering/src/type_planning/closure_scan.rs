//! Closure/lambda pre-scanning
//!
//! Pre-compute closure capture types from module-level statements and function
//! bodies. Handles decorator patterns, lambda parameter type hints for HOFs
//! (map/filter/reduce/sorted/min/max key=), and inline closure discovery.

use indexmap::IndexMap;
use pyaot_hir as hir;
use pyaot_types::Type;
use pyaot_utils::VarId;

use super::infer::extract_iterable_element_type;
use crate::Lowering;

impl<'a> Lowering<'a> {
    // ==================== Pre-computation Phase ====================

    /// Pre-compute closure capture types from module-level statements and function bodies.
    /// This must run before lowering functions so that lambda/closure type inference
    /// can use the captured variable types.
    ///
    /// §1.17b-d/f — all HIR functions, including the synthetic module-init
    /// function, are scanned through their CFG blocks in allocation order.
    pub(crate) fn precompute_closure_capture_types(&mut self, hir_module: &hir::Module) {
        // Fixpoint scan: lifted closures (e.g. `level3` in
        // outer→level1→level2→level3 chains) appear BEFORE their parent in
        // `hir_module.functions`, so a single top-to-bottom traversal cannot
        // see the parent's var_types when computing the child's captures.
        // Each pass populates `closure_capture_types` for every closure
        // expression encountered, seeding `var_types` from each function's
        // own previously-computed `closure_capture_types`. We iterate until
        // no slot improves (`Any → concrete`), capped by a small constant
        // so a pathological input cannot loop forever — Python closure
        // nesting in real code is at most a handful of levels deep, and
        // each pass strictly refines an `Any` slot to a concrete type.
        let max_passes = 16;
        for _ in 0..max_passes {
            let snapshot = self.closures.closure_capture_types.clone();
            for func_id in &hir_module.functions {
                if let Some(func) = hir_module.func_defs.get(func_id) {
                    let mut func_var_types: IndexMap<VarId, Type> = IndexMap::new();
                    // Seed from declared param types first.
                    for param in &func.params {
                        if let Some(ref ty) = param.ty {
                            func_var_types.insert(param.var, ty.clone());
                        }
                    }
                    // For lifted closures, also seed from this function's own
                    // closure_capture_types (computed by a parent's scan in
                    // an earlier pass). Matches captures to the leading
                    // positional params.
                    if let Some(capture_types) = self.get_closure_capture_types(func_id).cloned() {
                        for (i, ty) in capture_types.iter().enumerate() {
                            if let Some(param) = func.params.get(i) {
                                func_var_types
                                    .entry(param.var)
                                    .or_insert_with(|| ty.clone());
                            }
                        }
                    }
                    for block in func.blocks.values() {
                        for &stmt_id in &block.stmts {
                            self.scan_stmt_for_closures(stmt_id, hir_module, &mut func_var_types);
                        }
                        // Terminators carry exprs (cond / return value / raise
                        // exc+cause / yield value / iter-has-next) that can
                        // contain inline closures — scan them so
                        // `[x for x in <closure-producing expr>]` records its
                        // capture types.
                        self.scan_terminator_for_closures(
                            &block.terminator,
                            hir_module,
                            &mut func_var_types,
                        );
                    }
                }
            }
            if self.closures.closure_capture_types == snapshot {
                break;
            }
        }
    }

    /// Scan a HirTerminator for inline closures in any embedded exprs.
    fn scan_terminator_for_closures(
        &mut self,
        term: &hir::HirTerminator,
        hir_module: &hir::Module,
        var_types: &mut IndexMap<VarId, Type>,
    ) {
        use hir::HirTerminator::*;
        match term {
            Jump(_) | Unreachable | Reraise => {}
            Branch { cond, .. } => {
                let expr = &hir_module.exprs[*cond];
                self.scan_expr_for_closures(expr, hir_module, var_types);
            }
            Return(Some(expr_id)) | Yield { value: expr_id, .. } => {
                let expr = &hir_module.exprs[*expr_id];
                self.scan_expr_for_closures(expr, hir_module, var_types);
            }
            Return(None) => {}
            Raise { exc, cause } => {
                let expr = &hir_module.exprs[*exc];
                self.scan_expr_for_closures(expr, hir_module, var_types);
                if let Some(c) = cause {
                    let c_expr = &hir_module.exprs[*c];
                    self.scan_expr_for_closures(c_expr, hir_module, var_types);
                }
            }
        }
    }

    /// Scan a single straight-line statement for closure assignments and
    /// record capture types.
    fn scan_stmt_for_closures(
        &mut self,
        stmt_id: hir::StmtId,
        hir_module: &hir::Module,
        var_types: &mut IndexMap<VarId, Type>,
    ) {
        let stmt = &hir_module.stmts[stmt_id];
        match &stmt.kind {
            hir::StmtKind::Bind {
                target,
                value,
                type_hint,
            } => {
                let expr = &hir_module.exprs[*value];

                if let hir::BindingTarget::Var(target_var) = target {
                    // Determine the variable type (mirrors the Assign branch above)
                    let var_type = type_hint
                        .clone()
                        .unwrap_or_else(|| self.seed_infer_expr_type(expr, hir_module, var_types));
                    var_types.insert(*target_var, var_type);

                    // Scan the value expression for inline closures
                    self.scan_expr_for_closures(expr, hir_module, var_types);

                    // Check for decorated function pattern: var = decorator(FuncRef(func))
                    if let hir::ExprKind::Call {
                        func: call_func, ..
                    } = &expr.kind
                    {
                        if let Some(innermost_func_id) =
                            self.find_innermost_func_ref(expr, hir_module)
                        {
                            let call_func_expr = &hir_module.exprs[*call_func];
                            if let hir::ExprKind::FuncRef(decorator_func_id) = &call_func_expr.kind
                            {
                                if let Some(decorator_def) =
                                    hir_module.func_defs.get(decorator_func_id)
                                {
                                    if let Some(wrapper_func_id) =
                                        self.find_returned_closure(decorator_def, hir_module)
                                    {
                                        self.insert_wrapper_func_id(wrapper_func_id);
                                        self.closures
                                            .decorated_to_wrapper
                                            .insert(innermost_func_id, wrapper_func_id);
                                        if let Some(func_param) = decorator_def.params.first() {
                                            self.closures
                                                .wrapper_func_param_name
                                                .insert(wrapper_func_id, func_param.name);
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else {
                    // For Tuple/Attr/Index/ClassAttr targets, record each bound Var
                    // with Any type and scan the value for closures.
                    target.for_each_var(&mut |var_id| {
                        var_types.insert(var_id, Type::Any);
                    });
                    self.scan_expr_for_closures(expr, hir_module, var_types);
                }
            }
            hir::StmtKind::Expr(expr_id) => {
                // Scan expression statements (like function calls) for inline closures
                let expr = &hir_module.exprs[*expr_id];
                self.scan_expr_for_closures(expr, hir_module, var_types);
            }
            hir::StmtKind::Return(Some(expr_id)) => {
                // Scan return expressions for inline closures
                let expr = &hir_module.exprs[*expr_id];
                self.scan_expr_for_closures(expr, hir_module, var_types);
            }
            hir::StmtKind::Assert { cond, msg } => {
                self.scan_expr_for_closures(&hir_module.exprs[*cond], hir_module, var_types);
                if let Some(msg_id) = msg {
                    self.scan_expr_for_closures(&hir_module.exprs[*msg_id], hir_module, var_types);
                }
            }
            hir::StmtKind::IndexDelete { obj, index } => {
                self.scan_expr_for_closures(&hir_module.exprs[*obj], hir_module, var_types);
                self.scan_expr_for_closures(&hir_module.exprs[*index], hir_module, var_types);
            }
            hir::StmtKind::Raise { exc, cause } => {
                if let Some(exc_id) = exc {
                    self.scan_expr_for_closures(&hir_module.exprs[*exc_id], hir_module, var_types);
                }
                if let Some(cause_id) = cause {
                    self.scan_expr_for_closures(
                        &hir_module.exprs[*cause_id],
                        hir_module,
                        var_types,
                    );
                }
            }
            // §1.17b-d — `IterAdvance` replaces `ForBind` inside CFG blocks.
            // Preserve the Area G §G.10 loop-target element-type
            // propagation so closures **inside** the loop body see the
            // concrete type of the captured loop target (not `Any`).
            hir::StmtKind::IterAdvance { iter, target } => {
                let iter_expr = &hir_module.exprs[*iter];
                self.scan_expr_for_closures(iter_expr, hir_module, var_types);
                let iter_ty = self.seed_infer_expr_type(iter_expr, hir_module, var_types);
                let elem_ty = extract_iterable_element_type(&iter_ty);
                insert_target_types(target, &elem_ty, var_types);
            }
            hir::StmtKind::IterSetup { iter } => {
                self.scan_expr_for_closures(&hir_module.exprs[*iter], hir_module, var_types);
            }
            hir::StmtKind::Return(None)
            | hir::StmtKind::Break
            | hir::StmtKind::Continue
            | hir::StmtKind::Pass => {}
        }
    }

    /// Recursively scan an expression for inline closures and record their capture types
    fn scan_expr_for_closures(
        &mut self,
        expr: &hir::Expr,
        hir_module: &hir::Module,
        var_types: &IndexMap<VarId, Type>,
    ) {
        match &expr.kind {
            hir::ExprKind::Closure { func, captures } => {
                // Found an inline closure — record its capture types. Second
                // pass of `precompute_closure_capture_types` may refine an
                // earlier approximation: if the new inference produces a
                // strictly more concrete type than the one already stored,
                // overwrite. A strictly-monotone refinement rule
                // (`Any → concrete`) keeps the fixpoint trivially
                // convergent.
                let mut capture_types = Vec::new();
                for capture_id in captures {
                    let capture_expr = &hir_module.exprs[*capture_id];
                    let capture_type =
                        self.seed_infer_expr_type(capture_expr, hir_module, var_types);
                    capture_types.push(capture_type);
                }
                let refine = match self.get_closure_capture_types(func) {
                    None => true,
                    Some(existing) => {
                        existing.len() == capture_types.len()
                            && existing.iter().zip(capture_types.iter()).any(|(old, new)| {
                                matches!(old, Type::Any) && !matches!(new, Type::Any)
                            })
                    }
                };
                if refine {
                    self.insert_closure_capture_types(*func, capture_types);
                }
            }
            hir::ExprKind::Call {
                func, args, kwargs, ..
            } => {
                // Scan function and all arguments
                let func_expr = &hir_module.exprs[*func];
                self.scan_expr_for_closures(func_expr, hir_module, var_types);
                for call_arg in args {
                    let arg_id = match call_arg {
                        hir::CallArg::Regular(expr_id) | hir::CallArg::Starred(expr_id) => expr_id,
                    };
                    let arg_expr = &hir_module.exprs[*arg_id];
                    self.scan_expr_for_closures(arg_expr, hir_module, var_types);
                }
                for kw in kwargs {
                    let kw_expr = &hir_module.exprs[kw.value];
                    self.scan_expr_for_closures(kw_expr, hir_module, var_types);
                }
            }
            hir::ExprKind::BuiltinCall {
                builtin,
                args,
                kwargs,
                ..
            } => {
                // Register lambda parameter type hints for builtin HOFs
                // map/filter: callback takes 1 element parameter
                if matches!(builtin, hir::Builtin::Map | hir::Builtin::Filter) && args.len() >= 2 {
                    self.register_lambda_hints_from_iterable(
                        &hir_module.exprs[args[0]],
                        &hir_module.exprs[args[1]],
                        hir_module,
                        var_types,
                        1,
                        |elem| vec![elem],
                    );
                }
                // reduce: callback takes 2 element parameters (acc, elem)
                if matches!(builtin, hir::Builtin::Reduce) && args.len() >= 2 {
                    self.register_lambda_hints_from_iterable(
                        &hir_module.exprs[args[0]],
                        &hir_module.exprs[args[1]],
                        hir_module,
                        var_types,
                        2,
                        |elem| vec![elem.clone(), elem],
                    );
                }
                // sorted/min/max key=: key callback takes 1 element parameter
                if matches!(
                    builtin,
                    hir::Builtin::Sorted | hir::Builtin::Min | hir::Builtin::Max
                ) && !args.is_empty()
                {
                    let key_func = kwargs.iter().find_map(|kw| {
                        let kw_name = self.interner.resolve(kw.name);
                        if kw_name == "key" {
                            Some(&hir_module.exprs[kw.value])
                        } else {
                            None
                        }
                    });
                    if let Some(key_expr) = key_func {
                        self.register_lambda_hints_from_iterable(
                            key_expr,
                            &hir_module.exprs[args[0]],
                            hir_module,
                            var_types,
                            1,
                            |elem| vec![elem],
                        );
                    }
                }

                // Scan all arguments (this catches map(lambda ..., ...), filter(lambda ..., ...), etc.)
                for arg_id in args {
                    let arg_expr = &hir_module.exprs[*arg_id];
                    self.scan_expr_for_closures(arg_expr, hir_module, var_types);
                }
                for kw in kwargs {
                    let kw_expr = &hir_module.exprs[kw.value];
                    self.scan_expr_for_closures(kw_expr, hir_module, var_types);
                }
            }
            hir::ExprKind::MethodCall {
                obj,
                method,
                args,
                kwargs,
            } => {
                let obj_expr = &hir_module.exprs[*obj];
                self.scan_expr_for_closures(obj_expr, hir_module, var_types);

                // list.sort(key=...) — register lambda hints with the list's element type.
                if self.interner.resolve(*method) == "sort" {
                    let key_func = kwargs.iter().find_map(|kw| {
                        if self.interner.resolve(kw.name) == "key" {
                            Some(&hir_module.exprs[kw.value])
                        } else {
                            None
                        }
                    });
                    if let Some(key_expr) = key_func {
                        self.register_lambda_hints_from_iterable(
                            key_expr,
                            obj_expr,
                            hir_module,
                            var_types,
                            1,
                            |elem| vec![elem],
                        );
                    }
                }

                for arg_id in args {
                    let arg_expr = &hir_module.exprs[*arg_id];
                    self.scan_expr_for_closures(arg_expr, hir_module, var_types);
                }
                for kw in kwargs {
                    let kw_expr = &hir_module.exprs[kw.value];
                    self.scan_expr_for_closures(kw_expr, hir_module, var_types);
                }
            }
            hir::ExprKind::BinOp { left, right, .. }
            | hir::ExprKind::LogicalOp { left, right, .. } => {
                let left_expr = &hir_module.exprs[*left];
                let right_expr = &hir_module.exprs[*right];
                self.scan_expr_for_closures(left_expr, hir_module, var_types);
                self.scan_expr_for_closures(right_expr, hir_module, var_types);
            }
            hir::ExprKind::UnOp { operand, .. } => {
                let operand_expr = &hir_module.exprs[*operand];
                self.scan_expr_for_closures(operand_expr, hir_module, var_types);
            }
            hir::ExprKind::Compare { left, right, .. } => {
                let left_expr = &hir_module.exprs[*left];
                let right_expr = &hir_module.exprs[*right];
                self.scan_expr_for_closures(left_expr, hir_module, var_types);
                self.scan_expr_for_closures(right_expr, hir_module, var_types);
            }
            hir::ExprKind::List(elements)
            | hir::ExprKind::Tuple(elements)
            | hir::ExprKind::Set(elements) => {
                for elem_id in elements {
                    let elem_expr = &hir_module.exprs[*elem_id];
                    self.scan_expr_for_closures(elem_expr, hir_module, var_types);
                }
            }
            hir::ExprKind::Dict(pairs) => {
                for (key_id, val_id) in pairs {
                    let key_expr = &hir_module.exprs[*key_id];
                    let val_expr = &hir_module.exprs[*val_id];
                    self.scan_expr_for_closures(key_expr, hir_module, var_types);
                    self.scan_expr_for_closures(val_expr, hir_module, var_types);
                }
            }
            hir::ExprKind::Index { obj, index } => {
                let obj_expr = &hir_module.exprs[*obj];
                let index_expr = &hir_module.exprs[*index];
                self.scan_expr_for_closures(obj_expr, hir_module, var_types);
                self.scan_expr_for_closures(index_expr, hir_module, var_types);
            }
            hir::ExprKind::IfExpr {
                cond,
                then_val,
                else_val,
            } => {
                let cond_expr = &hir_module.exprs[*cond];
                let then_expr = &hir_module.exprs[*then_val];
                let else_expr = &hir_module.exprs[*else_val];
                self.scan_expr_for_closures(cond_expr, hir_module, var_types);
                self.scan_expr_for_closures(then_expr, hir_module, var_types);
                self.scan_expr_for_closures(else_expr, hir_module, var_types);
            }
            // Primitives and other simple expressions don't contain closures
            _ => {}
        }
    }

    // ==================== Lambda Hint Registration ====================

    /// Register lambda parameter type hints for a callback that takes elements from an iterable.
    /// Shared by map/filter (1 param), reduce (2 params), and sorted/min/max key= (1 param).
    pub(crate) fn register_lambda_hints_from_iterable(
        &mut self,
        func_expr: &hir::Expr,
        iterable_expr: &hir::Expr,
        hir_module: &hir::Module,
        var_types: &IndexMap<VarId, Type>,
        expected_non_capture: usize,
        make_hints: impl FnOnce(Type) -> Vec<Type>,
    ) {
        let iterable_type = self.seed_infer_expr_type(iterable_expr, hir_module, var_types);
        let elem_type = extract_iterable_element_type(&iterable_type);
        if matches!(elem_type, Type::Any) {
            return;
        }
        let func_info = match &func_expr.kind {
            hir::ExprKind::FuncRef(func_id) => Some((*func_id, vec![])),
            hir::ExprKind::Closure { func, captures } => Some((*func, captures.clone())),
            _ => None,
        };
        let Some((func_id, captures)) = func_info else {
            return;
        };
        let Some(func_def) = hir_module.func_defs.get(&func_id) else {
            return;
        };
        let num_non_capture = func_def.params.len().saturating_sub(captures.len());
        if num_non_capture != expected_non_capture {
            return;
        }
        let mut param_hints = Vec::new();
        for cap_id in &captures {
            let cap_expr = &hir_module.exprs[*cap_id];
            let cap_type = self.seed_infer_expr_type(cap_expr, hir_module, var_types);
            param_hints.push(cap_type);
        }
        param_hints.extend(make_hints(elem_type));
        self.insert_lambda_param_type_hints(func_id, param_hints);
    }

    // ==================== Nested Function Parameter Inference ====================

    /// Infer parameter types for nested / closure functions that lack
    /// explicit annotations, by joining the argument types observed at
    /// their call sites within the module.
    ///
    /// Target pattern (microgpt.py style):
    /// ```text
    /// def outer(root: Value):
    ///     def inner(v):           # no annotation, implicit `v: Any`
    ///         for child in v._children:
    ///             inner(child)
    ///     inner(root)             # call site #1 — v := Value
    /// ```
    ///
    /// Without this pass `v` stays `Any` and `v._children` fails with
    /// "cannot iterate over type 'Any'". With it, the call-site scan
    /// sees `inner(root)` passes `Value`, records a hint for `v`, and
    /// downstream prescan / return-type inference / lowering all see
    /// the concrete type.
    ///
    /// Algorithm:
    /// 1. First pass — scan the whole module and record, for every
    ///    `Bind { target: Var(v), value: Closure | FuncRef }`, the
    ///    mapping `VarId → (FuncId, capture-count)`. This lets step 2
    ///    resolve indirect `Call { func: Var(v), … }` back to the
    ///    underlying `FuncId`.
    /// 2. Second pass — scan every `Call` expression. For each call
    ///    whose target resolves (direct `FuncRef`, inline `Closure`,
    ///    or `Var` via the map from step 1), compute the inferred
    ///    type of every positional argument (using
    ///    `seed_infer_expr_type` with a param-overlay built from the
    ///    enclosing function's annotated parameters) and union it
    ///    into a per-`FuncId` accumulator keyed by positional index.
    /// 3. Finalisation — for every `FuncId` with collected types and
    ///    no existing `lambda_param_type_hints` entry AND no explicit
    ///    annotation on the target param, emit a `Vec<Type>` of
    ///    `capture-slots + inferred-arg-types`. The capture slots are
    ///    already carried by `closure_capture_types`; the hint covers
    ///    the non-capture positional params.
    ///
    /// Skipped intentionally:
    /// - Functions that already have explicit annotations on every
    ///   non-capture param (no win there).
    /// - Functions that already have a `lambda_param_type_hints`
    ///   entry from the HOF scan (map/filter/reduce) — those are
    ///   authoritative and this pass must not clobber them.
    /// - Starred / keyword / unpacked arguments (too fragile at this
    ///   stage; fall back to `Any`).
    pub(crate) fn infer_nested_function_param_types(&mut self, hir_module: &hir::Module) {
        use std::collections::HashMap;
        // 1. Build `var_to_func`: Bind-target VarIds that hold a Closure or
        //    FuncRef. Captures are stored so step 2 can offset past them.
        let mut var_to_func: HashMap<VarId, (pyaot_utils::FuncId, usize)> = HashMap::new();
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
                    var_to_func.insert(*var_id, (*func, captures.len()));
                }
                hir::ExprKind::FuncRef(func_id) => {
                    var_to_func.insert(*var_id, (*func_id, 0));
                }
                _ => {}
            }
        }

        // 2. Walk every function body, collecting per-FuncId
        //    positional arg-type accumulators.
        let mut accumulators: HashMap<pyaot_utils::FuncId, Vec<Type>> = HashMap::new();

        for (_fid, func) in hir_module.func_defs.iter() {
            // Build a param-overlay for the enclosing function — the
            // arg-type inference uses it so that `inner(self)` where
            // `self: Value` resolves the arg as `Value`, not `Any`.
            let mut overlay: IndexMap<VarId, Type> = IndexMap::new();
            for p in &func.params {
                if let Some(ref ty) = p.ty {
                    overlay.insert(p.var, ty.clone());
                }
            }
            // §1.17b-d — walk the CFG. Terminators carry exprs (Return /
            // Branch / Raise) that need scanning for embedded calls.
            for block in func.blocks.values() {
                for &stmt_id in &block.stmts {
                    self.collect_call_arg_types(
                        stmt_id,
                        hir_module,
                        &var_to_func,
                        &overlay,
                        &mut accumulators,
                    );
                }
                self.scan_terminator_for_calls(
                    &block.terminator,
                    hir_module,
                    &var_to_func,
                    &overlay,
                    &mut accumulators,
                );
            }
        }
        // 3. Commit hints for eligible functions.
        for (func_id, inferred) in accumulators {
            if self.get_lambda_param_type_hints(&func_id).is_some() {
                continue;
            }
            let Some(func_def) = hir_module.func_defs.get(&func_id) else {
                continue;
            };
            let capture_count = self
                .get_closure_capture_types(&func_id)
                .map(|v| v.len())
                .unwrap_or(0);
            let non_capture_params = func_def.params.len().saturating_sub(capture_count);
            if inferred.is_empty() || non_capture_params == 0 {
                continue;
            }
            // Skip if every non-capture param already carries an
            // explicit annotation — the user's types win.
            let all_annotated = func_def
                .params
                .iter()
                .skip(capture_count)
                .all(|p| p.ty.is_some());
            if all_annotated {
                continue;
            }
            let mut hint = Vec::with_capacity(func_def.params.len());
            if let Some(capture_types) = self.get_closure_capture_types(&func_id).cloned() {
                hint.extend(capture_types);
            } else {
                for _ in 0..capture_count {
                    hint.push(Type::Any);
                }
            }
            for i in 0..non_capture_params {
                let ty = inferred.get(i).cloned().unwrap_or(Type::Any);
                // Explicit annotation on this specific param still wins.
                let final_ty = func_def
                    .params
                    .get(capture_count + i)
                    .and_then(|p| p.ty.clone())
                    .unwrap_or(ty);
                hint.push(final_ty);
            }
            self.insert_lambda_param_type_hints(func_id, hint);
        }
    }

    /// Scan a HirTerminator's embedded exprs for resolved calls.
    fn scan_terminator_for_calls(
        &self,
        term: &hir::HirTerminator,
        hir_module: &hir::Module,
        var_to_func: &std::collections::HashMap<VarId, (pyaot_utils::FuncId, usize)>,
        overlay: &IndexMap<VarId, Type>,
        accumulators: &mut std::collections::HashMap<pyaot_utils::FuncId, Vec<Type>>,
    ) {
        use hir::HirTerminator::*;
        let exprs: Vec<hir::ExprId> = match term {
            Jump(_) | Unreachable | Reraise => Vec::new(),
            Branch { cond, .. } => vec![*cond],
            Return(Some(e)) | Yield { value: e, .. } => vec![*e],
            Return(None) => Vec::new(),
            Raise { exc, cause } => {
                let mut v = vec![*exc];
                if let Some(c) = cause {
                    v.push(*c);
                }
                v
            }
        };
        for expr_id in exprs {
            let expr = &hir_module.exprs[expr_id];
            self.scan_expr_for_calls(expr, hir_module, var_to_func, overlay, accumulators);
        }
    }

    /// Recursively scan a statement for `Call` expressions, resolving
    /// each to a target `FuncId` via direct `FuncRef`, inline
    /// `Closure`, or `Var`-through-`var_to_func`. For every resolved
    /// call, infer each positional-arg type (via
    /// `seed_infer_expr_type` with `overlay`) and union it into
    /// `accumulators[func_id][positional_index]`.
    fn collect_call_arg_types(
        &self,
        stmt_id: hir::StmtId,
        hir_module: &hir::Module,
        var_to_func: &std::collections::HashMap<VarId, (pyaot_utils::FuncId, usize)>,
        overlay: &IndexMap<VarId, Type>,
        accumulators: &mut std::collections::HashMap<pyaot_utils::FuncId, Vec<Type>>,
    ) {
        let stmt = &hir_module.stmts[stmt_id];
        match &stmt.kind {
            hir::StmtKind::Expr(expr_id) | hir::StmtKind::Return(Some(expr_id)) => {
                let expr = &hir_module.exprs[*expr_id];
                self.scan_expr_for_calls(expr, hir_module, var_to_func, overlay, accumulators);
            }
            hir::StmtKind::Bind { value, .. } => {
                let expr = &hir_module.exprs[*value];
                self.scan_expr_for_calls(expr, hir_module, var_to_func, overlay, accumulators);
            }
            hir::StmtKind::IterAdvance { iter, .. } | hir::StmtKind::IterSetup { iter } => {
                let expr = &hir_module.exprs[*iter];
                self.scan_expr_for_calls(expr, hir_module, var_to_func, overlay, accumulators);
            }
            hir::StmtKind::Raise { exc, cause } => {
                if let Some(expr_id) = exc {
                    let expr = &hir_module.exprs[*expr_id];
                    self.scan_expr_for_calls(expr, hir_module, var_to_func, overlay, accumulators);
                }
                if let Some(expr_id) = cause {
                    let expr = &hir_module.exprs[*expr_id];
                    self.scan_expr_for_calls(expr, hir_module, var_to_func, overlay, accumulators);
                }
            }
            hir::StmtKind::Assert { cond, msg } => {
                self.scan_expr_for_calls(
                    &hir_module.exprs[*cond],
                    hir_module,
                    var_to_func,
                    overlay,
                    accumulators,
                );
                if let Some(msg_id) = msg {
                    self.scan_expr_for_calls(
                        &hir_module.exprs[*msg_id],
                        hir_module,
                        var_to_func,
                        overlay,
                        accumulators,
                    );
                }
            }
            hir::StmtKind::IndexDelete { obj, index } => {
                self.scan_expr_for_calls(
                    &hir_module.exprs[*obj],
                    hir_module,
                    var_to_func,
                    overlay,
                    accumulators,
                );
                self.scan_expr_for_calls(
                    &hir_module.exprs[*index],
                    hir_module,
                    var_to_func,
                    overlay,
                    accumulators,
                );
            }
            hir::StmtKind::Return(None)
            | hir::StmtKind::Break
            | hir::StmtKind::Continue
            | hir::StmtKind::Pass => {}
        }
    }

    fn scan_expr_for_calls(
        &self,
        expr: &hir::Expr,
        hir_module: &hir::Module,
        var_to_func: &std::collections::HashMap<VarId, (pyaot_utils::FuncId, usize)>,
        overlay: &IndexMap<VarId, Type>,
        accumulators: &mut std::collections::HashMap<pyaot_utils::FuncId, Vec<Type>>,
    ) {
        if let hir::ExprKind::Call { func, args, .. } = &expr.kind {
            let func_expr = &hir_module.exprs[*func];
            let resolved = match &func_expr.kind {
                hir::ExprKind::FuncRef(fid) => Some((*fid, 0)),
                hir::ExprKind::Closure {
                    func: fid,
                    captures,
                } => Some((*fid, captures.len())),
                hir::ExprKind::Var(v) => var_to_func.get(v).copied(),
                _ => None,
            };
            if let Some((fid, capture_offset)) = resolved {
                let mut positional_tys: Vec<Type> = Vec::with_capacity(args.len());
                let mut skip = false;
                for call_arg in args {
                    match call_arg {
                        hir::CallArg::Regular(arg_id) => {
                            let arg_expr = &hir_module.exprs[*arg_id];
                            let ty = self.seed_infer_expr_type(arg_expr, hir_module, overlay);
                            positional_tys.push(ty);
                        }
                        hir::CallArg::Starred(_) => {
                            // Skip — starred args unpack variably.
                            skip = true;
                            break;
                        }
                    }
                }
                if !skip {
                    // Accumulator is per-positional-arg (0-indexed
                    // over the NON-capture positional params).
                    // `capture_offset` is tracked elsewhere via
                    // `get_closure_capture_types`; at the call site
                    // only positional args matter.
                    let _ = capture_offset;
                    let entry = accumulators
                        .entry(fid)
                        .or_insert_with(|| vec![Type::Never; positional_tys.len()]);
                    if entry.len() < positional_tys.len() {
                        entry.resize(positional_tys.len(), Type::Never);
                    }
                    for (i, ty) in positional_tys.into_iter().enumerate() {
                        // Join concrete observations; `Any` is a no-op
                        // so the first concrete arg-type wins over
                        // later `Any`s.
                        let existing = std::mem::replace(&mut entry[i], Type::Never);
                        entry[i] = join_nested_arg_ty(existing, ty);
                    }
                }
            }
            // Recurse into args so nested Calls get scanned too.
            for call_arg in args {
                let arg_id = match call_arg {
                    hir::CallArg::Regular(id) | hir::CallArg::Starred(id) => id,
                };
                let arg_expr = &hir_module.exprs[*arg_id];
                self.scan_expr_for_calls(arg_expr, hir_module, var_to_func, overlay, accumulators);
            }
            // Recurse into the func expression itself (for nested
            // closure factories).
            self.scan_expr_for_calls(func_expr, hir_module, var_to_func, overlay, accumulators);
            return;
        }
        // Generic recursion into sub-expressions for any other kind.
        match &expr.kind {
            hir::ExprKind::BinOp { left, right, .. }
            | hir::ExprKind::Compare { left, right, .. } => {
                self.scan_expr_for_calls(
                    &hir_module.exprs[*left],
                    hir_module,
                    var_to_func,
                    overlay,
                    accumulators,
                );
                self.scan_expr_for_calls(
                    &hir_module.exprs[*right],
                    hir_module,
                    var_to_func,
                    overlay,
                    accumulators,
                );
            }
            hir::ExprKind::UnOp { operand, .. } => {
                self.scan_expr_for_calls(
                    &hir_module.exprs[*operand],
                    hir_module,
                    var_to_func,
                    overlay,
                    accumulators,
                );
            }
            hir::ExprKind::LogicalOp { left, right, .. } => {
                self.scan_expr_for_calls(
                    &hir_module.exprs[*left],
                    hir_module,
                    var_to_func,
                    overlay,
                    accumulators,
                );
                self.scan_expr_for_calls(
                    &hir_module.exprs[*right],
                    hir_module,
                    var_to_func,
                    overlay,
                    accumulators,
                );
            }
            hir::ExprKind::IfExpr {
                cond,
                then_val,
                else_val,
            } => {
                self.scan_expr_for_calls(
                    &hir_module.exprs[*cond],
                    hir_module,
                    var_to_func,
                    overlay,
                    accumulators,
                );
                self.scan_expr_for_calls(
                    &hir_module.exprs[*then_val],
                    hir_module,
                    var_to_func,
                    overlay,
                    accumulators,
                );
                self.scan_expr_for_calls(
                    &hir_module.exprs[*else_val],
                    hir_module,
                    var_to_func,
                    overlay,
                    accumulators,
                );
            }
            hir::ExprKind::MethodCall { obj, args, .. } => {
                self.scan_expr_for_calls(
                    &hir_module.exprs[*obj],
                    hir_module,
                    var_to_func,
                    overlay,
                    accumulators,
                );
                for a in args {
                    self.scan_expr_for_calls(
                        &hir_module.exprs[*a],
                        hir_module,
                        var_to_func,
                        overlay,
                        accumulators,
                    );
                }
            }
            hir::ExprKind::BuiltinCall { args, .. } => {
                for a in args {
                    self.scan_expr_for_calls(
                        &hir_module.exprs[*a],
                        hir_module,
                        var_to_func,
                        overlay,
                        accumulators,
                    );
                }
            }
            _ => {}
        }
    }
}

/// Join two observed arg-types for the same positional slot across
/// distinct call sites. `Never` is the accumulator's empty seed.
/// `Any` provides no new information and is ignored. Otherwise pick
/// the concrete type; if they differ, fall back to `Any` (conservative).
fn join_nested_arg_ty(a: Type, b: Type) -> Type {
    match (a, b) {
        (Type::Never, x) | (x, Type::Never) => x,
        (Type::Any, x) | (x, Type::Any) => x,
        (a, b) if a == b => a,
        _ => Type::Any,
    }
}

/// Recursively walk a `BindingTarget`, inserting the destructured element
/// type into `var_types` for each `Var` leaf. Used by the `ForBind` arm of
/// `scan_stmt_for_closures` to propagate loop-target types so nested
/// closures capturing those targets infer concrete types (§G.10).
///
/// Mirrors the destructuring logic in `local_prescan::absorb_into_targets`
/// but without the loop-depth / loop-only bookkeeping, since this scanner
/// tracks only closure-capture types.
fn insert_target_types(
    target: &hir::BindingTarget,
    elem_ty: &Type,
    var_types: &mut IndexMap<VarId, Type>,
) {
    match target {
        hir::BindingTarget::Var(var_id) => {
            var_types.insert(*var_id, elem_ty.clone());
        }
        hir::BindingTarget::Tuple { elts, .. } => {
            if let Some(types) = elem_ty.tuple_elems() {
                if types.len() == elts.len() {
                    for (elt, t) in elts.iter().zip(types) {
                        insert_target_types(elt, t, var_types);
                    }
                } else {
                    for elt in elts {
                        insert_target_types(elt, &Type::Any, var_types);
                    }
                }
            } else if let Some(inner) = elem_ty.tuple_var_elem() {
                for elt in elts {
                    insert_target_types(elt, inner, var_types);
                }
            } else {
                for elt in elts {
                    insert_target_types(elt, &Type::Any, var_types);
                }
            }
        }
        hir::BindingTarget::Starred { inner, .. } => {
            // Starred captures a list of the outer element type.
            insert_target_types(inner, &Type::list_of(elem_ty.clone()), var_types);
        }
        hir::BindingTarget::Attr { .. }
        | hir::BindingTarget::Index { .. }
        | hir::BindingTarget::ClassAttr { .. } => {
            // Not a variable binding — nothing to record.
        }
    }
}
