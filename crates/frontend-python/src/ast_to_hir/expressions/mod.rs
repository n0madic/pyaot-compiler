//! Expression conversion from Python AST to HIR.
//!
//! Organized into submodules by expression category:
//! - `names`: Name resolution (variables, functions, classes, imports)
//! - `calls`: Function calls, method calls, stdlib calls
//! - `attributes`: Attribute access, module.attr, chained access
//! - `comparisons`: Compare, chained comparison desugaring
//! - `containers`: List, Dict, Set, Tuple literals, Subscript/Slice, BoolOp

mod attributes;
mod calls;
mod comparisons;
mod containers;
mod names;

use super::AstToHir;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::*;
use pyaot_types::Type;
use rustpython_parser::ast as py;

impl AstToHir {
    /// Helper to convert call arguments, handling starred arguments (*args)
    pub(crate) fn convert_call_args(&mut self, args: Vec<py::Expr>) -> Result<Vec<CallArg>> {
        let mut call_args = Vec::new();
        for arg in args {
            match arg {
                py::Expr::Starred(starred) => {
                    let expr_id = self.convert_expr(*starred.value)?;
                    call_args.push(CallArg::Starred(expr_id));
                }
                other => {
                    let expr_id = self.convert_expr(other)?;
                    call_args.push(CallArg::Regular(expr_id));
                }
            }
        }
        Ok(call_args)
    }

    pub(crate) fn convert_expr(&mut self, expr: py::Expr) -> Result<ExprId> {
        let expr_span = Self::span_from(&expr);
        let kind = match expr {
            py::Expr::Constant(c) => self.convert_constant(&c.value, expr_span)?,

            py::Expr::Name(name) => self.convert_name_expr(name, expr_span)?,

            py::Expr::BinOp(binop) => {
                let left = self.convert_expr(*binop.left)?;
                let right = self.convert_expr(*binop.right)?;
                let op = self.convert_binop(&binop.op, expr_span)?;
                ExprKind::BinOp { op, left, right }
            }

            py::Expr::UnaryOp(unop) => {
                let operand = self.convert_expr(*unop.operand)?;
                let op = self.convert_unop(&unop.op, expr_span)?;
                ExprKind::UnOp { op, operand }
            }

            py::Expr::Compare(cmp) => {
                return self.convert_compare_expr(cmp, expr_span);
            }

            py::Expr::Call(call) => {
                return self.convert_call_expr(call, expr_span);
            }

            py::Expr::IfExp(if_exp) => {
                let cond = self.convert_expr(*if_exp.test)?;
                let then_val = self.convert_expr(*if_exp.body)?;
                let else_val = self.convert_expr(*if_exp.orelse)?;
                ExprKind::IfExpr {
                    cond,
                    then_val,
                    else_val,
                }
            }

            py::Expr::List(list) => {
                return self.convert_list_expr(list, expr_span);
            }

            py::Expr::Tuple(tuple) => {
                return self.convert_tuple_expr(tuple, expr_span);
            }

            py::Expr::Dict(dict) => {
                return self.convert_dict_expr(dict, expr_span);
            }

            py::Expr::Set(set_expr) => {
                return self.convert_set_expr(set_expr, expr_span);
            }

            py::Expr::Subscript(sub) => {
                return self.convert_subscript_expr(sub, expr_span);
            }

            py::Expr::BoolOp(bool_op) => {
                return self.convert_boolop_expr(bool_op, expr_span);
            }

            py::Expr::JoinedStr(joined) => {
                return self.desugar_fstring(&joined.values, expr_span);
            }

            py::Expr::FormattedValue(_) => {
                return Err(CompilerError::parse_error(
                    "FormattedValue outside f-string",
                    expr_span,
                ));
            }

            py::Expr::Attribute(attr) => {
                return self.convert_attribute_expr(attr, expr_span);
            }

            py::Expr::Lambda(lambda) => {
                return self.convert_lambda(lambda);
            }

            py::Expr::ListComp(list_comp) => {
                return self.desugar_list_comprehension(list_comp);
            }

            py::Expr::DictComp(dict_comp) => {
                return self.desugar_dict_comprehension(dict_comp);
            }

            py::Expr::SetComp(set_comp) => {
                return self.desugar_set_comprehension(set_comp);
            }

            py::Expr::GeneratorExp(gen_exp) => {
                return self.desugar_generator_expression(gen_exp);
            }

            py::Expr::Yield(yield_expr) => {
                self.scope.current_func_is_generator = true;
                let value = if let Some(value_expr) = yield_expr.value {
                    Some(self.convert_expr(*value_expr)?)
                } else {
                    None
                };
                ExprKind::Yield(value)
            }

            py::Expr::YieldFrom(yield_from) => {
                self.scope.current_func_is_generator = true;

                let iter_expr_id = self.convert_expr(*yield_from.value)?;
                let temp_var = self.ids.alloc_var();

                let var_ref = self.module.exprs.alloc(Expr {
                    kind: ExprKind::Var(temp_var),
                    ty: None,
                    span: expr_span,
                });
                let yield_expr_id = self.module.exprs.alloc(Expr {
                    kind: ExprKind::Yield(Some(var_ref)),
                    ty: None,
                    span: expr_span,
                });
                let yield_stmt = self.module.stmts.alloc(Stmt {
                    kind: StmtKind::Expr(yield_expr_id),
                    span: expr_span,
                });
                let for_stmt = self.module.stmts.alloc(Stmt {
                    kind: StmtKind::For {
                        target: temp_var,
                        iter: iter_expr_id,
                        body: vec![yield_stmt],
                        else_block: vec![],
                    },
                    span: expr_span,
                });
                self.scope.pending_stmts.push(for_stmt);

                ExprKind::None
            }

            py::Expr::NamedExpr(named) => {
                let value_id = self.convert_expr(*named.value)?;
                let target_var = self.get_or_create_var_from_expr(&named.target)?;
                self.mark_var_initialized(&named.target);

                let assign_stmt = self.module.stmts.alloc(Stmt {
                    kind: StmtKind::Assign {
                        target: target_var,
                        value: value_id,
                        type_hint: None,
                    },
                    span: expr_span,
                });
                self.scope.pending_stmts.push(assign_stmt);

                ExprKind::Var(target_var)
            }

            _ => {
                return Err(CompilerError::parse_error(
                    format!("Unsupported expression: {:?}", expr),
                    expr_span,
                ));
            }
        };

        // Infer types for literal expressions
        let ty = match &kind {
            ExprKind::Int(_) => Some(Type::Int),
            ExprKind::Float(_) => Some(Type::Float),
            ExprKind::Bool(_) => Some(Type::Bool),
            ExprKind::Str(_) => Some(Type::Str),
            ExprKind::Bytes(_) => Some(Type::Bytes),
            ExprKind::None => Some(Type::None),
            _ => None,
        };

        let expr_id = self.module.exprs.alloc(Expr {
            kind,
            ty,
            span: expr_span,
        });
        Ok(expr_id)
    }
}
