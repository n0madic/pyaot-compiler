//! Comparison expressions: simple comparisons and chained comparison desugaring.

use super::super::AstToHir;
use pyaot_diagnostics::Result;
use pyaot_hir::{cfg_build::CfgStmt, *};
use pyaot_types::Type;
use pyaot_utils::Span;
use rustpython_parser::ast as py;

impl AstToHir {
    /// Convert a Compare expression. Handles both simple binary comparisons
    /// and chained comparisons like `a < b < c`.
    pub(crate) fn convert_compare_expr(
        &mut self,
        cmp: py::ExprCompare,
        expr_span: Span,
    ) -> Result<ExprId> {
        if cmp.ops.len() == 1 && cmp.comparators.len() == 1 {
            // Detect type(x) == <type_name> pattern before normal conversion.
            if matches!(cmp.ops[0], py::CmpOp::Eq | py::CmpOp::NotEq) {
                // type(x) == NAME
                let fwd = Self::detect_type_comparison(&cmp.left, &cmp.comparators[0]);
                if let Some(class_str) = fwd {
                    let left = self.convert_expr(*cmp.left)?;
                    let op = self.convert_cmpop(&cmp.ops[0], expr_span)?;
                    let right = self.module.exprs.alloc(Expr {
                        kind: ExprKind::Str(self.interner.intern(class_str)),
                        ty: Some(Type::Str),
                        span: expr_span,
                    });
                    return Ok(self.module.exprs.alloc(Expr {
                        kind: ExprKind::Compare { left, op, right },
                        ty: Some(Type::Bool),
                        span: expr_span,
                    }));
                }
                // NAME == type(x) (reversed)
                let rev = Self::detect_type_comparison(&cmp.comparators[0], &cmp.left);
                if let Some(class_str) = rev {
                    let comparator = cmp
                        .comparators
                        .into_iter()
                        .next()
                        .expect("comparison must have at least one comparator");
                    let left = self.convert_expr(comparator)?;
                    let op = self.convert_cmpop(&cmp.ops[0], expr_span)?;
                    let right = self.module.exprs.alloc(Expr {
                        kind: ExprKind::Str(self.interner.intern(class_str)),
                        ty: Some(Type::Str),
                        span: expr_span,
                    });
                    return Ok(self.module.exprs.alloc(Expr {
                        kind: ExprKind::Compare { left, op, right },
                        ty: Some(Type::Bool),
                        span: expr_span,
                    }));
                }
            }

            // Simple binary comparison
            let left = self.convert_expr(*cmp.left)?;
            let right = self.convert_expr(
                cmp.comparators
                    .into_iter()
                    .next()
                    .expect("comparison must have at least one comparator"),
            )?;
            let op = self.convert_cmpop(&cmp.ops[0], expr_span)?;

            Ok(self.module.exprs.alloc(Expr {
                kind: ExprKind::Compare { left, op, right },
                ty: None,
                span: expr_span,
            }))
        } else {
            // Chained comparison: desugar to AND chain
            self.desugar_chained_comparison(cmp, expr_span)
        }
    }

    /// Detect `type(x) == <type_name>` pattern.
    /// Returns the type class string (e.g., `"<class 'tuple'>"`) if `type_call` is a
    /// `type(arg)` call and `type_name` is a known built-in type name.
    fn detect_type_comparison(type_call: &py::Expr, type_name: &py::Expr) -> Option<&'static str> {
        let py::Expr::Call(call) = type_call else {
            return None;
        };
        let py::Expr::Name(func_name) = &*call.func else {
            return None;
        };
        if func_name.id.as_str() != "type" || call.args.len() != 1 || !call.keywords.is_empty() {
            return None;
        }
        let py::Expr::Name(name) = type_name else {
            return None;
        };
        match name.id.as_str() {
            "int" => Some("<class 'int'>"),
            "float" => Some("<class 'float'>"),
            "bool" => Some("<class 'bool'>"),
            "str" => Some("<class 'str'>"),
            "tuple" => Some("<class 'tuple'>"),
            "list" => Some("<class 'list'>"),
            "dict" => Some("<class 'dict'>"),
            "set" => Some("<class 'set'>"),
            "bytes" => Some("<class 'bytes'>"),
            _ => None,
        }
    }

    /// Desugar a chained comparison like `a < b < c` into `(a < b) and (b < c)`.
    /// Middle operands that may have side effects are stored in temp variables
    /// to ensure single evaluation.
    fn desugar_chained_comparison(
        &mut self,
        cmp: py::ExprCompare,
        expr_span: Span,
    ) -> Result<ExprId> {
        let mut operands: Vec<py::Expr> = Vec::with_capacity(cmp.comparators.len() + 1);
        operands.push(*cmp.left);
        operands.extend(cmp.comparators);

        let ops = cmp.ops;

        // First pass: convert all operands and create temp vars for middle ones
        let mut converted_operands: Vec<(ExprId, Option<pyaot_utils::VarId>)> =
            Vec::with_capacity(operands.len());

        for (i, operand) in operands.iter().enumerate() {
            let expr_id = self.convert_expr(operand.clone())?;

            let is_middle = i > 0 && i < ops.len();
            if is_middle && Self::expr_needs_temp_var(operand) {
                let temp_name = format!("__chain_{}", self.ids.next_comp_id);
                self.ids.next_comp_id += 1;

                let temp_var_id = self.ids.alloc_var();
                let temp_interned = self.interner.intern(&temp_name);
                self.symbols.var_map.insert(temp_interned, temp_var_id);

                let assign_stmt = self.module.stmts.alloc(Stmt {
                    kind: StmtKind::Bind {
                        target: BindingTarget::Var(temp_var_id),
                        value: expr_id,
                        type_hint: None,
                    },
                    span: expr_span,
                });
                self.scope.pending_stmts.push(CfgStmt::stmt(assign_stmt));

                converted_operands.push((expr_id, Some(temp_var_id)));
            } else {
                converted_operands.push((expr_id, None));
            }
        }

        // Second pass: create comparisons and chain them with AND
        let mut comparisons: Vec<ExprId> = Vec::with_capacity(ops.len());

        for (i, op) in ops.iter().enumerate() {
            let hir_op = self.convert_cmpop(op, expr_span)?;

            let left = if i > 0 {
                if let Some(temp_var) = converted_operands[i].1 {
                    self.module.exprs.alloc(Expr {
                        kind: ExprKind::Var(temp_var),
                        ty: None,
                        span: expr_span,
                    })
                } else {
                    converted_operands[i].0
                }
            } else {
                converted_operands[i].0
            };

            let right = if let Some(temp_var) = converted_operands[i + 1].1 {
                self.module.exprs.alloc(Expr {
                    kind: ExprKind::Var(temp_var),
                    ty: None,
                    span: expr_span,
                })
            } else {
                converted_operands[i + 1].0
            };

            let cmp_expr = self.module.exprs.alloc(Expr {
                kind: ExprKind::Compare {
                    left,
                    op: hir_op,
                    right,
                },
                ty: Some(Type::Bool),
                span: expr_span,
            });

            comparisons.push(cmp_expr);
        }

        // Chain all comparisons with AND
        let mut result = comparisons[0];
        for cmp_expr in comparisons.into_iter().skip(1) {
            result = self.module.exprs.alloc(Expr {
                kind: ExprKind::LogicalOp {
                    op: LogicalOp::And,
                    left: result,
                    right: cmp_expr,
                },
                ty: Some(Type::Bool),
                span: expr_span,
            });
        }

        Ok(result)
    }

    /// Check if an expression needs a temp variable to avoid multiple evaluation.
    fn expr_needs_temp_var(expr: &py::Expr) -> bool {
        !matches!(expr, py::Expr::Name(_) | py::Expr::Constant(_))
    }
}
