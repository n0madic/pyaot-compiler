//! Context manager support: With statement desugaring

use super::AstToHir;
use pyaot_diagnostics::Result;
use pyaot_hir::*;
use pyaot_types::Type;
use pyaot_utils::Span;
use rustpython_parser::ast as py;

impl AstToHir {
    pub(crate) fn convert_with(
        &mut self,
        with_stmt: py::StmtWith,
        _stmt_span: Span,
    ) -> Result<StmtId> {
        self.convert_with_stmt(with_stmt)
    }

    /// Convert a `with` statement by desugaring to try/except/finally.
    ///
    /// ```python
    /// with EXPR as VAR:
    ///     BODY
    /// ```
    ///
    /// Desugars to:
    /// ```python
    /// __ctx_mgr = EXPR
    /// __ctx_val = __ctx_mgr.__enter__()
    /// VAR = __ctx_val  # if 'as VAR' present
    /// __ctx_had_exc = False
    /// try:
    ///     BODY
    /// except:
    ///     __ctx_had_exc = True
    ///     __ctx_suppress = __ctx_mgr.__exit__(1, 0, 0)  # 1 = exception occurred
    ///     if not __ctx_suppress:
    ///         raise  # re-raise if not suppressed
    /// finally:
    ///     if not __ctx_had_exc:
    ///         __ctx_mgr.__exit__(0, 0, 0)  # 0 = no exception
    /// ```
    fn convert_with_stmt(&mut self, with_stmt: py::StmtWith) -> Result<StmtId> {
        let with_span = Self::span_from(&with_stmt);

        // Handle multiple items by nesting: with A, B: body → with A: with B: body
        if with_stmt.items.len() > 1 {
            let mut items = with_stmt.items;
            let last = items
                .pop()
                .expect("with statement must have at least one item");
            let inner_with = py::Stmt::With(py::StmtWith {
                items: vec![last],
                body: with_stmt.body,
                range: with_stmt.range,
                type_comment: None,
            });
            let outer_with = py::Stmt::With(py::StmtWith {
                items,
                body: vec![inner_with],
                range: with_stmt.range,
                type_comment: None,
            });
            return self.convert_stmt(outer_with);
        }

        // Single context manager case
        let item = &with_stmt.items[0];
        let ctx_id = self.next_ctx_id;
        self.next_ctx_id += 1;

        // 1. __ctx_mgr = EXPR
        let ctx_mgr_name = self.interner.intern(&format!("__ctx_mgr_{}", ctx_id));
        let ctx_mgr_var = self.alloc_var_id();
        self.var_map.insert(ctx_mgr_name, ctx_mgr_var);

        let context_expr = self.convert_expr(item.context_expr.clone())?;
        let ctx_mgr_assign = self.module.stmts.alloc(Stmt {
            kind: StmtKind::Assign {
                target: ctx_mgr_var,
                value: context_expr,
                type_hint: None,
            },
            span: with_span,
        });

        // 2. __ctx_val = __ctx_mgr.__enter__()
        let ctx_mgr_ref = self.module.exprs.alloc(Expr {
            kind: ExprKind::Var(ctx_mgr_var),
            ty: None,
            span: with_span,
        });
        let enter_call = self.module.exprs.alloc(Expr {
            kind: ExprKind::MethodCall {
                obj: ctx_mgr_ref,
                method: self.interner.intern("__enter__"),
                args: vec![],
                kwargs: vec![],
            },
            ty: None,
            span: with_span,
        });

        let ctx_val_name = self.interner.intern(&format!("__ctx_val_{}", ctx_id));
        let ctx_val_var = self.alloc_var_id();
        self.var_map.insert(ctx_val_name, ctx_val_var);

        let ctx_val_assign = self.module.stmts.alloc(Stmt {
            kind: StmtKind::Assign {
                target: ctx_val_var,
                value: enter_call,
                type_hint: None,
            },
            span: with_span,
        });

        // 3. VAR = __ctx_val (if 'as VAR' present)
        let target_assign = if let Some(ref opt_var) = item.optional_vars {
            let target_var = self.get_or_create_var_from_expr(opt_var)?;
            let ctx_val_ref = self.module.exprs.alloc(Expr {
                kind: ExprKind::Var(ctx_val_var),
                ty: None,
                span: with_span,
            });
            Some(self.module.stmts.alloc(Stmt {
                kind: StmtKind::Assign {
                    target: target_var,
                    value: ctx_val_ref,
                    type_hint: None,
                },
                span: with_span,
            }))
        } else {
            None
        };

        // 4. __ctx_had_exc = False
        let ctx_had_exc_name = self.interner.intern(&format!("__ctx_had_exc_{}", ctx_id));
        let ctx_had_exc_var = self.alloc_var_id();
        self.var_map.insert(ctx_had_exc_name, ctx_had_exc_var);

        let false_expr = self.module.exprs.alloc(Expr {
            kind: ExprKind::Bool(false),
            ty: Some(Type::Bool),
            span: with_span,
        });
        let ctx_had_exc_init = self.module.stmts.alloc(Stmt {
            kind: StmtKind::Assign {
                target: ctx_had_exc_var,
                value: false_expr,
                type_hint: Some(Type::Bool),
            },
            span: with_span,
        });

        // 5. __ctx_suppress variable (used in except handler)
        let ctx_suppress_name = self.interner.intern(&format!("__ctx_suppress_{}", ctx_id));
        let ctx_suppress_var = self.alloc_var_id();
        self.var_map.insert(ctx_suppress_name, ctx_suppress_var);

        // 6. Convert body statements
        let mut body = Vec::new();
        for stmt in with_stmt.body {
            let stmt_id = self.convert_stmt(stmt)?;
            let pending = self.take_pending_stmts();
            body.extend(pending);
            body.push(stmt_id);
        }

        // 7. Create except handler body:
        //    __ctx_had_exc = True
        //    __ctx_suppress = __ctx_mgr.__exit__(1, 0, 0)
        //    if not __ctx_suppress:
        //        raise

        // 7a. __ctx_had_exc = True
        let true_expr = self.module.exprs.alloc(Expr {
            kind: ExprKind::Bool(true),
            ty: Some(Type::Bool),
            span: with_span,
        });
        let ctx_had_exc_set = self.module.stmts.alloc(Stmt {
            kind: StmtKind::Assign {
                target: ctx_had_exc_var,
                value: true_expr,
                type_hint: None,
            },
            span: with_span,
        });

        // 7b. __ctx_suppress = __ctx_mgr.__exit__(1, 0, 0)
        let ctx_mgr_ref_except = self.module.exprs.alloc(Expr {
            kind: ExprKind::Var(ctx_mgr_var),
            ty: None,
            span: with_span,
        });
        let one = self.module.exprs.alloc(Expr {
            kind: ExprKind::Int(1),
            ty: Some(Type::Int),
            span: with_span,
        });
        let zero_exc1 = self.module.exprs.alloc(Expr {
            kind: ExprKind::Int(0),
            ty: Some(Type::Int),
            span: with_span,
        });
        let zero_exc2 = self.module.exprs.alloc(Expr {
            kind: ExprKind::Int(0),
            ty: Some(Type::Int),
            span: with_span,
        });
        let exit_call_exc = self.module.exprs.alloc(Expr {
            kind: ExprKind::MethodCall {
                obj: ctx_mgr_ref_except,
                method: self.interner.intern("__exit__"),
                args: vec![one, zero_exc1, zero_exc2],
                kwargs: vec![],
            },
            ty: None,
            span: with_span,
        });
        let ctx_suppress_assign = self.module.stmts.alloc(Stmt {
            kind: StmtKind::Assign {
                target: ctx_suppress_var,
                value: exit_call_exc,
                type_hint: Some(Type::Bool),
            },
            span: with_span,
        });

        // 7c. if not __ctx_suppress: raise
        let ctx_suppress_ref = self.module.exprs.alloc(Expr {
            kind: ExprKind::Var(ctx_suppress_var),
            ty: Some(Type::Bool),
            span: with_span,
        });
        let not_suppress = self.module.exprs.alloc(Expr {
            kind: ExprKind::UnOp {
                op: UnOp::Not,
                operand: ctx_suppress_ref,
            },
            ty: Some(Type::Bool),
            span: with_span,
        });
        let raise_stmt = self.module.stmts.alloc(Stmt {
            kind: StmtKind::Raise {
                exc: None,
                cause: None,
            }, // Bare raise re-raises current exception
            span: with_span,
        });
        let if_not_suppress = self.module.stmts.alloc(Stmt {
            kind: StmtKind::If {
                cond: not_suppress,
                then_block: vec![raise_stmt],
                else_block: vec![],
            },
            span: with_span,
        });

        let except_body = vec![ctx_had_exc_set, ctx_suppress_assign, if_not_suppress];

        // Create except handler (catches all exceptions)
        let handler = ExceptHandler {
            ty: None,   // catch all exceptions
            name: None, // don't bind exception
            body: except_body,
        };

        // 8. Create finally block:
        //    if not __ctx_had_exc:
        //        __ctx_mgr.__exit__(0, 0, 0)
        let ctx_had_exc_ref = self.module.exprs.alloc(Expr {
            kind: ExprKind::Var(ctx_had_exc_var),
            ty: Some(Type::Bool),
            span: with_span,
        });
        let not_had_exc = self.module.exprs.alloc(Expr {
            kind: ExprKind::UnOp {
                op: UnOp::Not,
                operand: ctx_had_exc_ref,
            },
            ty: Some(Type::Bool),
            span: with_span,
        });

        let ctx_mgr_ref_finally = self.module.exprs.alloc(Expr {
            kind: ExprKind::Var(ctx_mgr_var),
            ty: None,
            span: with_span,
        });
        // TODO: CPython passes (exc_type, exc_val, exc_tb) to __exit__.
        // We pass (0, 0, 0) for no exception and (1, 0, 0) for exception,
        // which means context managers that inspect their arguments will behave
        // incorrectly. Full support requires propagating exception info from the
        // runtime's exception handling mechanism.
        // When no exception occurs, pass 0 to __exit__ (0 represents "no exception")
        let zero1 = self.module.exprs.alloc(Expr {
            kind: ExprKind::Int(0),
            ty: Some(Type::Int),
            span: with_span,
        });
        let zero2 = self.module.exprs.alloc(Expr {
            kind: ExprKind::Int(0),
            ty: Some(Type::Int),
            span: with_span,
        });
        let zero3 = self.module.exprs.alloc(Expr {
            kind: ExprKind::Int(0),
            ty: Some(Type::Int),
            span: with_span,
        });
        let exit_call_finally = self.module.exprs.alloc(Expr {
            kind: ExprKind::MethodCall {
                obj: ctx_mgr_ref_finally,
                method: self.interner.intern("__exit__"),
                args: vec![zero1, zero2, zero3],
                kwargs: vec![],
            },
            ty: None,
            span: with_span,
        });
        let exit_stmt_finally = self.module.stmts.alloc(Stmt {
            kind: StmtKind::Expr(exit_call_finally),
            span: with_span,
        });

        let if_not_had_exc = self.module.stmts.alloc(Stmt {
            kind: StmtKind::If {
                cond: not_had_exc,
                then_block: vec![exit_stmt_finally],
                else_block: vec![],
            },
            span: with_span,
        });

        // 9. Create try/except/finally
        let try_stmt = self.module.stmts.alloc(Stmt {
            kind: StmtKind::Try {
                body,
                handlers: vec![handler],
                else_block: vec![],
                finally_block: vec![if_not_had_exc],
            },
            span: with_span,
        });

        // 10. Build statement sequence: return first, add rest to pending
        // The pending_stmts mechanism injects statements before the returned one
        self.pending_stmts.push(ctx_mgr_assign);
        self.pending_stmts.push(ctx_val_assign);
        if let Some(assign) = target_assign {
            self.pending_stmts.push(assign);
        }
        self.pending_stmts.push(ctx_had_exc_init);

        Ok(try_stmt)
    }
}
