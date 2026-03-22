use super::AstToHir;
use pyaot_diagnostics::Result;
use pyaot_hir::*;
use pyaot_types::Type;
use pyaot_utils::Span;
use pyaot_utils::VarId;
use rustpython_parser::ast as py;

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
        let temp_name = format!("__comp_{}", self.next_comp_id);
        self.next_comp_id += 1;

        // 2. Save outer scope for scope isolation
        let outer_var_map = self.var_map.clone();

        // 3. Create temp variable and register it
        let temp_var_id = self.alloc_var_id();
        let temp_interned = self.interner.intern(&temp_name);
        self.var_map.insert(temp_interned, temp_var_id);

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
            kind: StmtKind::Assign {
                target: temp_var_id,
                value: empty_list,
                type_hint: None,
            },
            span: comp_span,
        });

        // 5. Generate nested for-loops with append calls
        let loop_stmts = self.generate_list_comprehension_loop(
            &comp.generators,
            0,
            &comp.elt,
            temp_var_id,
            comp_span,
        )?;

        // 6. Add init statement and loop statements to pending_stmts
        self.pending_stmts.push(init_stmt);
        self.pending_stmts.extend(loop_stmts);

        // 7. Restore outer scope but keep temp var visible
        self.var_map = outer_var_map;
        self.var_map.insert(temp_interned, temp_var_id);

        // 8. Return reference to temp variable
        Ok(self.module.exprs.alloc(Expr {
            kind: ExprKind::Var(temp_var_id),
            ty: None,
            span: comp_span,
        }))
    }

    /// Recursively generate for-loops for list comprehension generators.
    fn generate_list_comprehension_loop(
        &mut self,
        generators: &[py::Comprehension],
        gen_idx: usize,
        elt_expr: &py::Expr,
        result_var: VarId,
        comp_span: Span,
    ) -> Result<Vec<StmtId>> {
        if gen_idx >= generators.len() {
            // Base case: generate append statement
            // __comp_N.append(elt_expr)
            let elt_id = self.convert_expr(elt_expr.clone())?;

            // Create reference to result list
            let list_ref = self.module.exprs.alloc(Expr {
                kind: ExprKind::Var(result_var),
                ty: None,
                span: comp_span,
            });

            // Create append method call
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

            // Wrap in expression statement
            let append_stmt = self.module.stmts.alloc(Stmt {
                kind: StmtKind::Expr(append_call),
                span: comp_span,
            });

            return Ok(vec![append_stmt]);
        }

        let gen = &generators[gen_idx];

        // Create loop target variable
        let target_var = self.get_or_create_var_from_expr(&gen.target)?;

        // Convert iterable
        let iter_expr = self.convert_expr(gen.iter.clone())?;

        // Recursively generate inner body
        let mut inner_body = self.generate_list_comprehension_loop(
            generators,
            gen_idx + 1,
            elt_expr,
            result_var,
            comp_span,
        )?;

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

        // Create for loop
        let for_stmt = self.module.stmts.alloc(Stmt {
            kind: StmtKind::For {
                target: target_var,
                iter: iter_expr,
                body: inner_body,
                else_block: Vec::new(),
            },
            span: comp_span,
        });

        Ok(vec![for_stmt])
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
        let temp_name = format!("__comp_{}", self.next_comp_id);
        self.next_comp_id += 1;

        // 2. Save outer scope for scope isolation
        let outer_var_map = self.var_map.clone();

        // 3. Create temp variable and register it
        let temp_var_id = self.alloc_var_id();
        let temp_interned = self.interner.intern(&temp_name);
        self.var_map.insert(temp_interned, temp_var_id);

        // 4. Create empty dict initialization: __comp_N = {}
        let empty_dict = self.module.exprs.alloc(Expr {
            kind: ExprKind::Dict(vec![]),
            ty: None,
            span: comp_span,
        });
        let init_stmt = self.module.stmts.alloc(Stmt {
            kind: StmtKind::Assign {
                target: temp_var_id,
                value: empty_dict,
                type_hint: None,
            },
            span: comp_span,
        });

        // 5. Generate nested for-loops with dict set operations
        let loop_stmts = self.generate_dict_comprehension_loop(
            &comp.generators,
            0,
            &comp.key,
            &comp.value,
            temp_var_id,
            comp_span,
        )?;

        // 6. Add init statement and loop statements to pending_stmts
        self.pending_stmts.push(init_stmt);
        self.pending_stmts.extend(loop_stmts);

        // 7. Restore outer scope but keep temp var visible
        self.var_map = outer_var_map;
        self.var_map.insert(temp_interned, temp_var_id);

        // 8. Return reference to temp variable
        Ok(self.module.exprs.alloc(Expr {
            kind: ExprKind::Var(temp_var_id),
            ty: None,
            span: comp_span,
        }))
    }

    /// Recursively generate for-loops for dict comprehension generators.
    fn generate_dict_comprehension_loop(
        &mut self,
        generators: &[py::Comprehension],
        gen_idx: usize,
        key_expr: &py::Expr,
        value_expr: &py::Expr,
        result_var: VarId,
        comp_span: Span,
    ) -> Result<Vec<StmtId>> {
        if gen_idx >= generators.len() {
            // Base case: generate dict assignment
            // __comp_N[key] = value
            let key_id = self.convert_expr(key_expr.clone())?;
            let value_id = self.convert_expr(value_expr.clone())?;

            // Create reference to result dict
            let dict_ref = self.module.exprs.alloc(Expr {
                kind: ExprKind::Var(result_var),
                ty: None,
                span: comp_span,
            });

            // Create index assignment statement
            let set_stmt = self.module.stmts.alloc(Stmt {
                kind: StmtKind::IndexAssign {
                    obj: dict_ref,
                    index: key_id,
                    value: value_id,
                },
                span: comp_span,
            });

            return Ok(vec![set_stmt]);
        }

        let gen = &generators[gen_idx];

        // Create loop target variable
        let target_var = self.get_or_create_var_from_expr(&gen.target)?;

        // Convert iterable
        let iter_expr = self.convert_expr(gen.iter.clone())?;

        // Recursively generate inner body
        let mut inner_body = self.generate_dict_comprehension_loop(
            generators,
            gen_idx + 1,
            key_expr,
            value_expr,
            result_var,
            comp_span,
        )?;

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

        // Create for loop
        let for_stmt = self.module.stmts.alloc(Stmt {
            kind: StmtKind::For {
                target: target_var,
                iter: iter_expr,
                body: inner_body,
                else_block: Vec::new(),
            },
            span: comp_span,
        });

        Ok(vec![for_stmt])
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
        let temp_name = format!("__comp_{}", self.next_comp_id);
        self.next_comp_id += 1;

        // 2. Save outer scope for scope isolation
        let outer_var_map = self.var_map.clone();

        // 3. Create temp variable and register it
        let temp_var_id = self.alloc_var_id();
        let temp_interned = self.interner.intern(&temp_name);
        self.var_map.insert(temp_interned, temp_var_id);

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
            kind: StmtKind::Assign {
                target: temp_var_id,
                value: empty_set,
                type_hint: None,
            },
            span: comp_span,
        });

        // 5. Generate nested for-loops with set.add() operations
        let loop_stmts = self.generate_set_comprehension_loop(
            &comp.generators,
            0,
            &comp.elt,
            temp_var_id,
            comp_span,
        )?;

        // 6. Add init statement and loop statements to pending_stmts
        self.pending_stmts.push(init_stmt);
        self.pending_stmts.extend(loop_stmts);

        // 7. Restore outer scope but keep temp var visible
        self.var_map = outer_var_map;
        self.var_map.insert(temp_interned, temp_var_id);

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

        // 1. Generate unique function name
        let func_name = format!("__genexp_{}", self.next_comp_id);
        self.next_comp_id += 1;

        // 2. Save outer scope for scope isolation
        let outer_var_map = self.var_map.clone();

        // 3. Generate the for-loop body with yield
        let body_stmts = self.generate_generator_expression_loop(
            &genexp.generators,
            0,
            &genexp.elt,
            genexp_span,
        )?;

        // 4. Restore outer scope
        self.var_map = outer_var_map;

        // 5. Create the generator function
        let func_id = self.alloc_func_id();
        let func_name_interned = self.interner.intern(&func_name);

        let gen_func = Function {
            id: func_id,
            name: func_name_interned,
            params: vec![],
            return_type: None,
            body: body_stmts,
            span: genexp_span,
            cell_vars: std::collections::HashSet::new(),
            nonlocal_vars: std::collections::HashSet::new(),
            is_generator: true,
            method_kind: MethodKind::default(), // Generator expressions are not methods
            is_abstract: false,                 // Generator expressions cannot be abstract
        };

        // 6. Register the function
        self.module.func_defs.insert(func_id, gen_func);
        self.module.functions.push(func_id);

        // 7. Create a call to the generator function: __genexp_N()
        let func_ref = self.module.exprs.alloc(Expr {
            kind: ExprKind::FuncRef(func_id),
            ty: None,
            span: genexp_span,
        });

        let call_expr = self.module.exprs.alloc(Expr {
            kind: ExprKind::Call {
                func: func_ref,
                args: vec![],
                kwargs: vec![],
                kwargs_unpack: None,
            },
            ty: None,
            span: genexp_span,
        });

        Ok(call_expr)
    }

    /// Recursively generate for-loops for generator expression.
    fn generate_generator_expression_loop(
        &mut self,
        generators: &[py::Comprehension],
        gen_idx: usize,
        yield_expr: &py::Expr,
        genexp_span: Span,
    ) -> Result<Vec<StmtId>> {
        if gen_idx >= generators.len() {
            // Base case: generate yield statement
            let yield_value = self.convert_expr(yield_expr.clone())?;

            // Create yield expression
            let yield_expr_id = self.module.exprs.alloc(Expr {
                kind: ExprKind::Yield(Some(yield_value)),
                ty: None,
                span: genexp_span,
            });

            // Wrap in expression statement
            let yield_stmt = self.module.stmts.alloc(Stmt {
                kind: StmtKind::Expr(yield_expr_id),
                span: genexp_span,
            });

            return Ok(vec![yield_stmt]);
        }

        let gen = &generators[gen_idx];

        // Create loop target variable
        let target_var = self.get_or_create_var_from_expr(&gen.target)?;

        // Convert iterable
        let iter_expr = self.convert_expr(gen.iter.clone())?;

        // Recursively generate inner body
        let mut inner_body = self.generate_generator_expression_loop(
            generators,
            gen_idx + 1,
            yield_expr,
            genexp_span,
        )?;

        // Wrap in if conditions (innermost to outermost)
        for cond in gen.ifs.iter().rev() {
            let cond_expr = self.convert_expr(cond.clone())?;
            let if_stmt = self.module.stmts.alloc(Stmt {
                kind: StmtKind::If {
                    cond: cond_expr,
                    then_block: inner_body,
                    else_block: vec![],
                },
                span: genexp_span,
            });
            inner_body = vec![if_stmt];
        }

        // Create for loop
        let for_stmt = self.module.stmts.alloc(Stmt {
            kind: StmtKind::For {
                target: target_var,
                iter: iter_expr,
                body: inner_body,
                else_block: Vec::new(),
            },
            span: genexp_span,
        });

        Ok(vec![for_stmt])
    }

    /// Recursively generate for-loops for set comprehension generators.
    fn generate_set_comprehension_loop(
        &mut self,
        generators: &[py::Comprehension],
        gen_idx: usize,
        elem_expr: &py::Expr,
        result_var: VarId,
        comp_span: Span,
    ) -> Result<Vec<StmtId>> {
        if gen_idx >= generators.len() {
            // Base case: generate set.add(elem)
            let elem_id = self.convert_expr(elem_expr.clone())?;

            // Create reference to result set
            let set_ref = self.module.exprs.alloc(Expr {
                kind: ExprKind::Var(result_var),
                ty: None,
                span: comp_span,
            });

            // Create method name interned string
            let add_method = self.interner.intern("add");

            // Create set.add(elem) method call expression
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

            // Create expression statement for the method call
            let add_stmt = self.module.stmts.alloc(Stmt {
                kind: StmtKind::Expr(method_call),
                span: comp_span,
            });

            return Ok(vec![add_stmt]);
        }

        let gen = &generators[gen_idx];

        // Create loop target variable
        let target_var = self.get_or_create_var_from_expr(&gen.target)?;

        // Convert iterable
        let iter_expr = self.convert_expr(gen.iter.clone())?;

        // Recursively generate inner body
        let mut inner_body = self.generate_set_comprehension_loop(
            generators,
            gen_idx + 1,
            elem_expr,
            result_var,
            comp_span,
        )?;

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

        // Create for loop
        let for_stmt = self.module.stmts.alloc(Stmt {
            kind: StmtKind::For {
                target: target_var,
                iter: iter_expr,
                body: inner_body,
                else_block: Vec::new(),
            },
            span: comp_span,
        });

        Ok(vec![for_stmt])
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
                // Arithmetic on ints produces int (except division)
                if matches!(op.op, py::Operator::Div | py::Operator::Pow) {
                    return false;
                }
                self.is_int_expression(&op.left) && self.is_int_expression(&op.right)
            }
            py::Expr::Name(_) => {
                // Loop variables from range() are ints
                true // Conservative: assume names are ints when all sources are int
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

    /// Check if an iterable expression produces integers (e.g., range()).
    fn is_int_iterable(&self, expr: &py::Expr) -> bool {
        match expr {
            py::Expr::Call(call) => {
                if let py::Expr::Name(name) = call.func.as_ref() {
                    matches!(name.id.as_str(), "range")
                } else {
                    false
                }
            }
            _ => false,
        }
    }
}
