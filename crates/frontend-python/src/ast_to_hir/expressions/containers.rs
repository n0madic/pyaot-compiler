//! Container literal expressions: List, Tuple, Dict (with unpacking), Set, Subscript/Slice.

use super::super::AstToHir;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::{cfg_build::CfgStmt, *};
use pyaot_utils::Span;
use rustpython_parser::ast as py;

impl AstToHir {
    /// Convert a List literal expression.
    pub(crate) fn convert_list_expr(
        &mut self,
        list: py::ExprList,
        expr_span: Span,
    ) -> Result<ExprId> {
        let mut elements = Vec::new();
        for elem in list.elts {
            elements.push(self.convert_expr(elem)?);
        }
        Ok(self.module.exprs.alloc(Expr {
            kind: ExprKind::List(elements),
            ty: None,
            span: expr_span,
        }))
    }

    /// Convert a Tuple literal expression.
    pub(crate) fn convert_tuple_expr(
        &mut self,
        tuple: py::ExprTuple,
        expr_span: Span,
    ) -> Result<ExprId> {
        let mut elements = Vec::new();
        for elem in tuple.elts {
            elements.push(self.convert_expr(elem)?);
        }
        Ok(self.module.exprs.alloc(Expr {
            kind: ExprKind::Tuple(elements),
            ty: None,
            span: expr_span,
        }))
    }

    /// Convert a Dict literal expression, including dict unpacking (`{**d1, **d2}`).
    pub(crate) fn convert_dict_expr(
        &mut self,
        dict: py::ExprDict,
        expr_span: Span,
    ) -> Result<ExprId> {
        let has_unpacking = dict.keys.iter().any(|k| k.is_none());

        if !has_unpacking {
            // Fast path: no unpacking, convert directly
            let mut pairs = Vec::new();
            for (key, value) in dict.keys.into_iter().zip(dict.values.into_iter()) {
                let key_expr =
                    self.convert_expr(key.expect("checked: no unpacking in fast path"))?;
                let value_expr = self.convert_expr(value)?;
                pairs.push((key_expr, value_expr));
            }
            return Ok(self.module.exprs.alloc(Expr {
                kind: ExprKind::Dict(pairs),
                ty: None,
                span: expr_span,
            }));
        }

        // Desugar dict unpacking:
        // {"a": 1, **d1, "b": 2, **d2} becomes:
        //   __dict_N = {"a": 1}      (leading regular pairs)
        //   __dict_N.update(d1)
        //   __dict_N["b"] = 2
        //   __dict_N.update(d2)
        //   result: __dict_N

        // 1. Generate unique temp var
        let temp_name = format!("__dict_{}", self.ids.next_comp_id);
        self.ids.next_comp_id += 1;
        let temp_var_id = self.ids.alloc_var();
        let temp_interned = self.interner.intern(&temp_name);
        self.symbols.var_map.insert(temp_interned, temp_var_id);

        // 2. Collect leading regular pairs for the initial dict
        let mut init_pairs = Vec::new();
        let mut items: Vec<(Option<py::Expr>, py::Expr)> =
            dict.keys.into_iter().zip(dict.values).collect();
        let mut start_idx = 0;
        for (key, value) in &items {
            if let Some(key) = key {
                let key_expr = self.convert_expr(key.clone())?;
                let value_expr = self.convert_expr(value.clone())?;
                init_pairs.push((key_expr, value_expr));
                start_idx += 1;
            } else {
                break;
            }
        }

        // 3. Create init: __dict_N = {leading pairs...}
        let init_dict = self.module.exprs.alloc(Expr {
            kind: ExprKind::Dict(init_pairs),
            ty: None,
            span: expr_span,
        });
        let init_stmt = self.module.stmts.alloc(Stmt {
            kind: StmtKind::Bind {
                target: BindingTarget::Var(temp_var_id),
                value: init_dict,
                type_hint: None,
            },
            span: expr_span,
        });
        self.scope.pending_stmts.push(CfgStmt::stmt(init_stmt));

        // 4. Process remaining items
        let remaining = items.split_off(start_idx);
        let update_str = self.interner.intern("update");
        for (key, value) in remaining {
            let dict_ref = self.module.exprs.alloc(Expr {
                kind: ExprKind::Var(temp_var_id),
                ty: None,
                span: expr_span,
            });

            if let Some(k) = key {
                // Regular pair: __dict_N[key] = value
                let key_expr = self.convert_expr(k)?;
                let value_expr = self.convert_expr(value)?;
                let assign_stmt = self.module.stmts.alloc(Stmt {
                    kind: StmtKind::Bind {
                        target: BindingTarget::Index {
                            obj: dict_ref,
                            index: key_expr,
                            span: expr_span,
                        },
                        value: value_expr,
                        type_hint: None,
                    },
                    span: expr_span,
                });
                self.scope.pending_stmts.push(CfgStmt::stmt(assign_stmt));
            } else {
                // Unpacking: __dict_N.update(value)
                let value_expr = self.convert_expr(value)?;
                let call_expr = self.module.exprs.alloc(Expr {
                    kind: ExprKind::MethodCall {
                        obj: dict_ref,
                        method: update_str,
                        args: vec![value_expr],
                        kwargs: vec![],
                    },
                    ty: None,
                    span: expr_span,
                });
                let call_stmt = self.module.stmts.alloc(Stmt {
                    kind: StmtKind::Expr(call_expr),
                    span: expr_span,
                });
                self.scope.pending_stmts.push(CfgStmt::stmt(call_stmt));
            }
        }

        // 5. Return reference to temp variable
        Ok(self.module.exprs.alloc(Expr {
            kind: ExprKind::Var(temp_var_id),
            ty: None,
            span: expr_span,
        }))
    }

    /// Convert a Set literal expression.
    pub(crate) fn convert_set_expr(
        &mut self,
        set_expr: py::ExprSet,
        expr_span: Span,
    ) -> Result<ExprId> {
        let mut elements = Vec::new();
        for elem in set_expr.elts {
            elements.push(self.convert_expr(elem)?);
        }
        Ok(self.module.exprs.alloc(Expr {
            kind: ExprKind::Set(elements),
            ty: None,
            span: expr_span,
        }))
    }

    /// Convert a Subscript expression (indexing or slicing).
    pub(crate) fn convert_subscript_expr(
        &mut self,
        sub: py::ExprSubscript,
        expr_span: Span,
    ) -> Result<ExprId> {
        let obj = self.convert_expr(*sub.value)?;

        let kind = match *sub.slice {
            py::Expr::Slice(slice) => {
                let start = if let Some(lower) = slice.lower {
                    Some(self.convert_expr(*lower)?)
                } else {
                    None
                };
                let end = if let Some(upper) = slice.upper {
                    Some(self.convert_expr(*upper)?)
                } else {
                    None
                };
                let step = if let Some(step_expr) = slice.step {
                    Some(self.convert_expr(*step_expr)?)
                } else {
                    None
                };
                ExprKind::Slice {
                    obj,
                    start,
                    end,
                    step,
                }
            }
            other => {
                let index = self.convert_expr(other)?;
                ExprKind::Index { obj, index }
            }
        };

        Ok(self.module.exprs.alloc(Expr {
            kind,
            ty: None,
            span: expr_span,
        }))
    }

    /// Convert a BoolOp expression (and/or chains).
    pub(crate) fn convert_boolop_expr(
        &mut self,
        bool_op: py::ExprBoolOp,
        expr_span: Span,
    ) -> Result<ExprId> {
        if bool_op.values.len() < 2 {
            return Err(CompilerError::parse_error(
                "BoolOp must have at least 2 values",
                expr_span,
            ));
        }

        let op = match bool_op.op {
            py::BoolOp::And => LogicalOp::And,
            py::BoolOp::Or => LogicalOp::Or,
        };

        let mut iter = bool_op.values.into_iter();
        let first =
            self.convert_expr(iter.next().expect("BoolOp must have at least two values"))?;
        let second =
            self.convert_expr(iter.next().expect("BoolOp must have at least two values"))?;

        let mut result_id = self.module.exprs.alloc(Expr {
            kind: ExprKind::LogicalOp {
                op,
                left: first,
                right: second,
            },
            ty: None,
            span: expr_span,
        });

        for val in iter {
            let next_val = self.convert_expr(val)?;
            result_id = self.module.exprs.alloc(Expr {
                kind: ExprKind::LogicalOp {
                    op,
                    left: result_id,
                    right: next_val,
                },
                ty: None,
                span: expr_span,
            });
        }

        Ok(result_id)
    }
}
