use super::AstToHir;
use pyaot_diagnostics::Result;
use pyaot_hir::*;
use pyaot_types::Type;
use pyaot_utils::InternedString;
use pyaot_utils::Span;
use pyaot_utils::VarId;
use rustpython_parser::ast as py;
use std::collections::HashSet;

/// Describes what action to perform at the innermost level of a comprehension loop.
enum ComprehensionAction<'a> {
    /// List comprehension: append element to result list
    ListAppend {
        elt: &'a py::Expr,
        result_var: VarId,
    },
    /// Dict comprehension: set key-value pair in result dict
    DictSet {
        key: &'a py::Expr,
        value: &'a py::Expr,
        result_var: VarId,
    },
    /// Set comprehension: add element to result set
    SetAdd {
        elt: &'a py::Expr,
        result_var: VarId,
    },
    /// Generator expression: yield element
    Yield { elt: &'a py::Expr },
}

impl AstToHir {
    /// Desugar a list comprehension into equivalent loop constructs.
    /// [x * 2 for x in range(5) if x > 1] becomes:
    /// __comp_N = []
    /// for x in range(5):
    ///     if x > 1:
    ///         __comp_N.append(x * 2)
    /// # result is __comp_N
    pub(crate) fn desugar_list_comprehension(&mut self, comp: py::ExprListComp) -> Result<ExprId> {
        let comp_span = Self::span_from(&comp);
        // 1. Generate unique temp var name
        let temp_name = format!("__comp_{}", self.ids.next_comp_id);
        self.ids.next_comp_id += 1;

        // 2. Save outer scope for scope isolation
        let outer_var_map = self.symbols.var_map.clone();

        // 2.1 Python 3 comprehension scoping: loop targets introduce
        // comp-local bindings that MUST NOT share VarIds with outer-
        // scope names. If an outer `for a, b in …` already mapped
        // `a`/`b` to VarIds and we reused them, a prescan-driven
        // union would see the same VarId written twice with
        // incompatible types (outer + inner) and first-write-wins
        // would record the wrong type. Forget comp-target names
        // before the loop is lowered so `bind_target` allocates
        // fresh VarIds.
        self.forget_comp_target_names(&comp.generators);

        // 3. Create temp variable and register it
        let temp_var_id = self.ids.alloc_var();
        let temp_interned = self.interner.intern(&temp_name);
        self.symbols.var_map.insert(temp_interned, temp_var_id);

        // 4. Create empty list initialization: __comp_N: list[T] = []
        //    Try to infer element type from the comprehension to set the correct
        //    elem_tag on the list. This avoids storing raw ints with ELEM_HEAP_OBJ
        //    tag, which causes GC warnings and potential corruption in debug builds.
        let elem_type = self.infer_comprehension_elem_type(&comp.elt, &comp.generators);
        let list_type = elem_type.map(|et| Type::List(Box::new(et)));
        let empty_list = self.module.exprs.alloc(Expr {
            kind: ExprKind::List(vec![]),
            ty: list_type.clone(),
            span: comp_span,
        });
        let init_stmt = self.module.stmts.alloc(Stmt {
            kind: StmtKind::Bind {
                target: BindingTarget::Var(temp_var_id),
                value: empty_list,
                type_hint: None,
            },
            span: comp_span,
        });

        // 5. Generate nested for-loops with append calls
        let action = ComprehensionAction::ListAppend {
            elt: &comp.elt,
            result_var: temp_var_id,
        };
        let loop_stmts =
            self.generate_comprehension_loop(&comp.generators, 0, &action, comp_span)?;

        // 6. Add init statement and loop statements to pending_stmts
        self.scope.pending_stmts.push(init_stmt);
        self.scope.pending_stmts.extend(loop_stmts);

        // 7. Restore outer scope but keep temp var visible
        self.symbols.var_map = outer_var_map;
        self.symbols.var_map.insert(temp_interned, temp_var_id);

        // 8. Return reference to temp variable
        Ok(self.module.exprs.alloc(Expr {
            kind: ExprKind::Var(temp_var_id),
            ty: None,
            span: comp_span,
        }))
    }

    /// Desugar a dict comprehension into equivalent loop constructs.
    /// {x: x*2 for x in range(5) if x > 1} becomes:
    /// __comp_N = {}
    /// for x in range(5):
    ///     if x > 1:
    ///         __comp_N[x] = x * 2
    /// # result is __comp_N
    pub(crate) fn desugar_dict_comprehension(&mut self, comp: py::ExprDictComp) -> Result<ExprId> {
        let comp_span = Self::span_from(&comp);

        // 1. Generate unique temp var name
        let temp_name = format!("__comp_{}", self.ids.next_comp_id);
        self.ids.next_comp_id += 1;

        // 2. Save outer scope for scope isolation
        let outer_var_map = self.symbols.var_map.clone();

        // 2.1 Python 3 comp scoping — see list-comp counterpart.
        self.forget_comp_target_names(&comp.generators);

        // 3. Create temp variable and register it
        let temp_var_id = self.ids.alloc_var();
        let temp_interned = self.interner.intern(&temp_name);
        self.symbols.var_map.insert(temp_interned, temp_var_id);

        // 4. Create empty dict initialization: __comp_N = {}
        let empty_dict = self.module.exprs.alloc(Expr {
            kind: ExprKind::Dict(vec![]),
            ty: None,
            span: comp_span,
        });
        let init_stmt = self.module.stmts.alloc(Stmt {
            kind: StmtKind::Bind {
                target: BindingTarget::Var(temp_var_id),
                value: empty_dict,
                type_hint: None,
            },
            span: comp_span,
        });

        // 5. Generate nested for-loops with dict set operations
        let action = ComprehensionAction::DictSet {
            key: &comp.key,
            value: &comp.value,
            result_var: temp_var_id,
        };
        let loop_stmts =
            self.generate_comprehension_loop(&comp.generators, 0, &action, comp_span)?;

        // 6. Add init statement and loop statements to pending_stmts
        self.scope.pending_stmts.push(init_stmt);
        self.scope.pending_stmts.extend(loop_stmts);

        // 7. Restore outer scope but keep temp var visible
        self.symbols.var_map = outer_var_map;
        self.symbols.var_map.insert(temp_interned, temp_var_id);

        // 8. Return reference to temp variable
        Ok(self.module.exprs.alloc(Expr {
            kind: ExprKind::Var(temp_var_id),
            ty: None,
            span: comp_span,
        }))
    }

    /// Desugar a set comprehension {expr for x in iterable if cond} into:
    /// __comp_N = set()
    /// for x in iterable:
    ///     if cond:
    ///         __comp_N.add(expr)
    /// __comp_N
    pub(crate) fn desugar_set_comprehension(&mut self, comp: py::ExprSetComp) -> Result<ExprId> {
        let comp_span = Self::span_from(&comp);

        // 1. Generate unique temp var name
        let temp_name = format!("__comp_{}", self.ids.next_comp_id);
        self.ids.next_comp_id += 1;

        // 2. Save outer scope for scope isolation
        let outer_var_map = self.symbols.var_map.clone();

        // 2.1 Python 3 comp scoping — see list-comp counterpart.
        self.forget_comp_target_names(&comp.generators);

        // 3. Create temp variable and register it
        let temp_var_id = self.ids.alloc_var();
        let temp_interned = self.interner.intern(&temp_name);
        self.symbols.var_map.insert(temp_interned, temp_var_id);

        // 4. Create empty set initialization: __comp_N = set()
        let empty_set = self.module.exprs.alloc(Expr {
            kind: ExprKind::BuiltinCall {
                builtin: Builtin::Set,
                args: vec![],
                kwargs: Vec::new(),
            },
            ty: Some(Type::Set(Box::new(Type::Any))),
            span: comp_span,
        });
        let init_stmt = self.module.stmts.alloc(Stmt {
            kind: StmtKind::Bind {
                target: BindingTarget::Var(temp_var_id),
                value: empty_set,
                type_hint: None,
            },
            span: comp_span,
        });

        // 5. Generate nested for-loops with set.add() operations
        let action = ComprehensionAction::SetAdd {
            elt: &comp.elt,
            result_var: temp_var_id,
        };
        let loop_stmts =
            self.generate_comprehension_loop(&comp.generators, 0, &action, comp_span)?;

        // 6. Add init statement and loop statements to pending_stmts
        self.scope.pending_stmts.push(init_stmt);
        self.scope.pending_stmts.extend(loop_stmts);

        // 7. Restore outer scope but keep temp var visible
        self.symbols.var_map = outer_var_map;
        self.symbols.var_map.insert(temp_interned, temp_var_id);

        // 8. Return reference to temp variable
        Ok(self.module.exprs.alloc(Expr {
            kind: ExprKind::Var(temp_var_id),
            ty: None,
            span: comp_span,
        }))
    }

    /// Desugar a generator expression (x for x in iterable if cond) into:
    /// def __genexp_N():
    ///     for x in iterable:
    ///         if cond:
    ///             yield x
    /// __genexp_N()
    pub(crate) fn desugar_generator_expression(
        &mut self,
        genexp: py::ExprGeneratorExp,
    ) -> Result<ExprId> {
        let genexp_span = Self::span_from(&genexp);

        // 1. Collect names introduced by loop targets — these are locals in the
        //    gen-expr function and must not count as captures.
        let mut local_scope: HashSet<String> = HashSet::new();
        for gen in &genexp.generators {
            self.add_target_to_scope(&gen.target, &mut local_scope);
        }

        // 2. Collect free variables used in the gen-expr (iters, conditions,
        //    and the yielded element). Anything that isn't a loop target becomes
        //    a candidate capture.
        let mut free_vars: Vec<InternedString> = Vec::new();
        for gen in &genexp.generators {
            self.collect_free_variables(&gen.iter, &local_scope, &mut free_vars);
            for cond in &gen.ifs {
                self.collect_free_variables(cond, &local_scope, &mut free_vars);
            }
        }
        self.collect_free_variables(&genexp.elt, &local_scope, &mut free_vars);

        // 3. Partition free vars: module globals keep their outer VarId mapping
        //    (accessed via rt_global_get_*); real captures become implicit
        //    leading params passed from the call site.
        //
        //    `module.globals` is only populated in `finalize_module`, so check
        //    `scope.module_level_assignments` (progressively populated as each
        //    module-level assignment completes) plus `module.globals` for
        //    anything already finalized. Without this, module-level vars would
        //    be treated as captures, which loses the deep-type propagation the
        //    global path provides (zip-of-lists → `rt_tuple_get_int`).
        let (global_propagation, captured_names): (Vec<_>, Vec<_>) =
            free_vars.into_iter().partition(|name| {
                if let Some(&var_id) = self.symbols.var_map.get(name) {
                    self.module.globals.contains(&var_id)
                        || self.scope.module_level_assignments.contains(&var_id)
                } else {
                    false
                }
            });

        // 4. Generate unique function name.
        let func_name = format!("__genexp_{}", self.ids.next_comp_id);
        self.ids.next_comp_id += 1;

        // 5. Save outer scope; build a fresh scope for the gen-expr body.
        let outer_var_map = std::mem::take(&mut self.symbols.var_map);
        let outer_global_vars = std::mem::take(&mut self.scope.global_vars);

        // 5.1 Propagate module globals — they use global storage, not captures.
        for name in &global_propagation {
            if let Some(&var_id) = outer_var_map.get(name) {
                self.symbols.var_map.insert(*name, var_id);
                self.scope.global_vars.insert(*name);
            }
        }

        // 5.2 Build implicit capture params and remap the captured names in the
        //     new scope so body references resolve to the capture VarIds.
        let mut params: Vec<Param> = Vec::new();
        let mut captures_outer_ids: Vec<VarId> = Vec::new();
        for captured_name in &captured_names {
            let outer_var_id = outer_var_map
                .get(captured_name)
                .copied()
                .expect("internal error: gen-expr captured variable not found in outer var_map");
            let capture_param_name = self.interner.intern(&format!(
                "__capture_{}",
                self.interner.resolve(*captured_name)
            ));
            let param_id = self.ids.alloc_var();
            self.symbols.var_map.insert(*captured_name, param_id);

            params.push(Param {
                name: capture_param_name,
                var: param_id,
                ty: None,
                default: None,
                kind: ParamKind::Regular,
                span: genexp_span,
            });
            captures_outer_ids.push(outer_var_id);
        }

        // 6. Generate the for-loop body with yield.
        let action = ComprehensionAction::Yield { elt: &genexp.elt };
        let body_stmts =
            self.generate_comprehension_loop(&genexp.generators, 0, &action, genexp_span)?;

        // 7. Restore outer scope.
        self.scope.global_vars = outer_global_vars;
        self.symbols.var_map = outer_var_map;

        // 8. Create the generator function.
        let func_id = self.ids.alloc_func();
        let func_name_interned = self.interner.intern(&func_name);

        let (blocks, entry_block) = cfg_build::build_cfg_from_tree(&body_stmts, &self.module.stmts);
        let gen_func = Function {
            id: func_id,
            name: func_name_interned,
            params,
            return_type: None,
            body: body_stmts,
            span: genexp_span,
            cell_vars: std::collections::HashSet::new(),
            nonlocal_vars: std::collections::HashSet::new(),
            is_generator: true,
            method_kind: MethodKind::default(),
            is_abstract: false,
            blocks,
            entry_block,
        };

        // 9. Register the function.
        self.module.func_defs.insert(func_id, gen_func);
        self.module.functions.push(func_id);

        // 10. Build call target: `FuncRef` if no captures, otherwise
        //     `Closure { func, captures }` so lowering plumbs capture types
        //     through the existing closure mechanism (see `lower_closure_call`
        //     in `expressions/calls/direct.rs`). That gives each capture param
        //     the concrete outer-var type instead of defaulting to `Any`.
        let call_target = if captures_outer_ids.is_empty() {
            self.module.exprs.alloc(Expr {
                kind: ExprKind::FuncRef(func_id),
                ty: None,
                span: genexp_span,
            })
        } else {
            let capture_exprs: Vec<ExprId> = captures_outer_ids
                .iter()
                .map(|outer_id| {
                    self.module.exprs.alloc(Expr {
                        kind: ExprKind::Var(*outer_id),
                        ty: None,
                        span: genexp_span,
                    })
                })
                .collect();
            self.module.exprs.alloc(Expr {
                kind: ExprKind::Closure {
                    func: func_id,
                    captures: capture_exprs,
                },
                ty: None,
                span: genexp_span,
            })
        };

        let call_expr = self.module.exprs.alloc(Expr {
            kind: ExprKind::Call {
                func: call_target,
                args: vec![],
                kwargs: vec![],
                kwargs_unpack: None,
            },
            ty: None,
            span: genexp_span,
        });

        Ok(call_expr)
    }

    /// Unified recursive loop generator for all comprehension types.
    /// The `action` parameter determines what happens at the innermost level.
    fn generate_comprehension_loop(
        &mut self,
        generators: &[py::Comprehension],
        gen_idx: usize,
        action: &ComprehensionAction<'_>,
        comp_span: Span,
    ) -> Result<Vec<StmtId>> {
        if gen_idx >= generators.len() {
            return self.generate_comprehension_base_case(action, comp_span);
        }

        let gen = &generators[gen_idx];

        // Build the unified binding target for this generator clause. Unlike
        // the old `get_or_create_var_from_expr`, `bind_target` accepts any
        // valid Python LHS — simple names, attribute/subscript leaves, and
        // nested/starred tuple patterns — matching the grammar CPython
        // admits inside `for TARGET in ITER:` of a comprehension.
        let target = self.bind_target(&gen.target)?;

        // Convert iterable
        let iter_expr = self.convert_expr(gen.iter.clone())?;

        // Recursively generate inner body
        let mut inner_body =
            self.generate_comprehension_loop(generators, gen_idx + 1, action, comp_span)?;

        // Wrap in if conditions (innermost to outermost)
        for cond in gen.ifs.iter().rev() {
            let cond_expr = self.convert_expr(cond.clone())?;
            let if_stmt = self.module.stmts.alloc(Stmt {
                kind: StmtKind::If {
                    cond: cond_expr,
                    then_block: inner_body,
                    else_block: vec![],
                },
                span: comp_span,
            });
            inner_body = vec![if_stmt];
        }

        // Emit the unified `ForBind` directly with the binding target.
        //
        // KNOWN LIMITATION: a *generator expression* (not list/dict/set
        // comprehension) with a non-`Var` target — e.g.
        // `sum(x * y for x, y in zip(a, b))` — desugars into a generator
        // function whose body is exactly this `ForBind { target: Tuple{..} }`.
        // The generator-desugaring pipeline's `detect_for_loop_generator`
        // (`crates/lowering/src/generators/for_loop.rs`) only recognises
        // `Var` targets for the optimised resume path; tuple targets fall
        // through to the generic sequential resume, which does not iterate
        // and therefore yields at most one element. List/dict/set
        // comprehensions (handled directly via `lower_for_bind`) do work
        // correctly for tuple targets. The full fix requires teaching the
        // resume builder to emit tuple unpacking with proper element-type
        // inference across `zip()`/`enumerate()` and friends — tracked as a
        // follow-up to the overall BindingTarget migration.
        let for_stmt = self.module.stmts.alloc(Stmt {
            kind: StmtKind::ForBind {
                target,
                iter: iter_expr,
                body: inner_body,
                else_block: Vec::new(),
            },
            span: comp_span,
        });

        Ok(vec![for_stmt])
    }

    /// Generate the base case statements for a comprehension's innermost body.
    fn generate_comprehension_base_case(
        &mut self,
        action: &ComprehensionAction<'_>,
        comp_span: Span,
    ) -> Result<Vec<StmtId>> {
        match action {
            ComprehensionAction::ListAppend { elt, result_var } => {
                let elt_id = self.convert_expr((*elt).clone())?;
                let list_ref = self.module.exprs.alloc(Expr {
                    kind: ExprKind::Var(*result_var),
                    ty: None,
                    span: comp_span,
                });
                let append_name = self.interner.intern("append");
                let append_call = self.module.exprs.alloc(Expr {
                    kind: ExprKind::MethodCall {
                        obj: list_ref,
                        method: append_name,
                        args: vec![elt_id],
                        kwargs: vec![],
                    },
                    ty: None,
                    span: comp_span,
                });
                let append_stmt = self.module.stmts.alloc(Stmt {
                    kind: StmtKind::Expr(append_call),
                    span: comp_span,
                });
                Ok(vec![append_stmt])
            }
            ComprehensionAction::DictSet {
                key,
                value,
                result_var,
            } => {
                let key_id = self.convert_expr((*key).clone())?;
                let value_id = self.convert_expr((*value).clone())?;
                let dict_ref = self.module.exprs.alloc(Expr {
                    kind: ExprKind::Var(*result_var),
                    ty: None,
                    span: comp_span,
                });
                let set_stmt = self.module.stmts.alloc(Stmt {
                    kind: StmtKind::Bind {
                        target: BindingTarget::Index {
                            obj: dict_ref,
                            index: key_id,
                            span: comp_span,
                        },
                        value: value_id,
                        type_hint: None,
                    },
                    span: comp_span,
                });
                Ok(vec![set_stmt])
            }
            ComprehensionAction::SetAdd { elt, result_var } => {
                let elem_id = self.convert_expr((*elt).clone())?;
                let set_ref = self.module.exprs.alloc(Expr {
                    kind: ExprKind::Var(*result_var),
                    ty: None,
                    span: comp_span,
                });
                let add_method = self.interner.intern("add");
                let method_call = self.module.exprs.alloc(Expr {
                    kind: ExprKind::MethodCall {
                        obj: set_ref,
                        method: add_method,
                        args: vec![elem_id],
                        kwargs: vec![],
                    },
                    ty: Some(Type::None),
                    span: comp_span,
                });
                let add_stmt = self.module.stmts.alloc(Stmt {
                    kind: StmtKind::Expr(method_call),
                    span: comp_span,
                });
                Ok(vec![add_stmt])
            }
            ComprehensionAction::Yield { elt } => {
                let yield_value = self.convert_expr((*elt).clone())?;
                let yield_expr_id = self.module.exprs.alloc(Expr {
                    kind: ExprKind::Yield(Some(yield_value)),
                    ty: None,
                    span: comp_span,
                });
                let yield_stmt = self.module.stmts.alloc(Stmt {
                    kind: StmtKind::Expr(yield_expr_id),
                    span: comp_span,
                });
                Ok(vec![yield_stmt])
            }
        }
    }

    /// Remove any names that would be introduced by the comprehension's
    /// `for TARGET in …` clauses from the active `var_map`, so that
    /// `bind_target` allocates fresh VarIds for each comp-local binding.
    /// Called right after saving `outer_var_map` in list/dict/set comp
    /// desugaring; the outer map is restored at the end of the comp, so
    /// these forgets are transparent to surrounding code.
    fn forget_comp_target_names(&mut self, generators: &[py::Comprehension]) {
        let mut target_names: std::collections::HashSet<String> = std::collections::HashSet::new();
        for gen in generators {
            self.add_target_to_scope(&gen.target, &mut target_names);
        }
        for name in target_names {
            let interned = self.interner.intern(&name);
            self.symbols.var_map.remove(&interned);
        }
    }

    /// Try to infer the element type of a list comprehension from its element
    /// expression and generators. Returns Some(Type::Int) when we can determine
    /// the result is integral, None otherwise (falls back to List(Any)).
    fn infer_comprehension_elem_type(
        &self,
        elt: &py::Expr,
        generators: &[py::Comprehension],
    ) -> Option<Type> {
        // Check if all generators iterate over integer sources (range)
        let all_int_sources = generators.iter().all(|gen| self.is_int_iterable(&gen.iter));

        if all_int_sources && self.is_int_expression(elt) {
            Some(Type::Int)
        } else {
            None
        }
    }

    /// Check if an expression is likely to produce an integer value.
    fn is_int_expression(&self, expr: &py::Expr) -> bool {
        match expr {
            py::Expr::Constant(c) => matches!(c.value, py::Constant::Int(_)),
            py::Expr::UnaryOp(op) => self.is_int_expression(&op.operand),
            py::Expr::BinOp(op) => {
                // Arithmetic on ints produces int (except true division which returns float)
                if matches!(op.op, py::Operator::Div) {
                    return false;
                }
                self.is_int_expression(&op.left) && self.is_int_expression(&op.right)
            }
            py::Expr::Name(name) => {
                // Only assume int if this name is a comprehension loop variable
                // (gated by all_int_sources check in the caller).
                // Check that the name isn't from an outer scope by verifying
                // it doesn't already exist in the var_map (loop variables are
                // created fresh during comprehension desugaring).
                let interned = self.interner.lookup(&name.id);
                match interned {
                    Some(s) => !self.symbols.var_map.contains_key(&s),
                    None => true, // Name not yet interned = likely a new loop variable
                }
            }
            py::Expr::Call(call) => {
                if let py::Expr::Name(name) = call.func.as_ref() {
                    matches!(name.id.as_str(), "int" | "abs" | "len" | "ord" | "hash")
                } else {
                    false
                }
            }
            py::Expr::IfExp(if_expr) => {
                self.is_int_expression(&if_expr.body) && self.is_int_expression(&if_expr.orelse)
            }
            _ => false,
        }
    }

    /// Check if an iterable expression produces integers (e.g., range(), list of ints).
    fn is_int_iterable(&self, expr: &py::Expr) -> bool {
        match expr {
            py::Expr::Call(call) => {
                if let py::Expr::Name(name) = call.func.as_ref() {
                    matches!(name.id.as_str(), "range")
                } else {
                    false
                }
            }
            // List literal with all int elements: [1, 2, 3]
            py::Expr::List(list) => list.elts.iter().all(|e| self.is_int_expression(e)),
            // Tuple literal with all int elements: (1, 2, 3)
            py::Expr::Tuple(tuple) => tuple.elts.iter().all(|e| self.is_int_expression(e)),
            _ => false,
        }
    }
}
