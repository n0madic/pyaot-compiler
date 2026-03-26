use super::AstToHir;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::*;
use pyaot_utils::Span;
use rustpython_parser::ast as py;

impl AstToHir {
    pub(crate) fn convert_constant(
        &mut self,
        constant: &py::Constant,
        span: Span,
    ) -> Result<ExprKind> {
        Ok(match constant {
            py::Constant::Int(i) => {
                // Handle big integers
                let val = if let Ok(v) = i.to_string().parse::<i64>() {
                    v
                } else {
                    return Err(CompilerError::parse_error(
                        "Integer literal out of i64 range",
                        span,
                    ));
                };
                ExprKind::Int(val)
            }
            py::Constant::Float(f) => ExprKind::Float(*f),
            py::Constant::Bool(b) => ExprKind::Bool(*b),
            py::Constant::Str(s) => {
                let interned = self.interner.intern(s);
                ExprKind::Str(interned)
            }
            py::Constant::Bytes(b) => ExprKind::Bytes(b.clone()),
            py::Constant::None => ExprKind::None,
            _ => {
                return Err(CompilerError::parse_error(
                    format!("Unsupported constant: {:?}", constant),
                    span,
                ))
            }
        })
    }

    pub(crate) fn convert_binop(&self, op: &py::Operator, span: Span) -> Result<BinOp> {
        Ok(match op {
            py::Operator::Add => BinOp::Add,
            py::Operator::Sub => BinOp::Sub,
            py::Operator::Mult => BinOp::Mul,
            py::Operator::Div => BinOp::Div,
            py::Operator::FloorDiv => BinOp::FloorDiv,
            py::Operator::Mod => BinOp::Mod,
            py::Operator::Pow => BinOp::Pow,
            // Bitwise operators
            py::Operator::BitAnd => BinOp::BitAnd,
            py::Operator::BitOr => BinOp::BitOr,
            py::Operator::BitXor => BinOp::BitXor,
            py::Operator::LShift => BinOp::LShift,
            py::Operator::RShift => BinOp::RShift,
            _ => {
                return Err(CompilerError::parse_error(
                    format!("Unsupported operator: {:?}", op),
                    span,
                ))
            }
        })
    }

    pub(crate) fn convert_unop(&self, op: &py::UnaryOp, _span: Span) -> Result<UnOp> {
        Ok(match op {
            py::UnaryOp::USub => UnOp::Neg,
            py::UnaryOp::Not => UnOp::Not,
            py::UnaryOp::Invert => UnOp::Invert, // Bitwise NOT (~)
            py::UnaryOp::UAdd => UnOp::Pos,      // Unary plus (+)
        })
    }

    pub(crate) fn convert_cmpop(&self, op: &py::CmpOp, _span: Span) -> Result<CmpOp> {
        Ok(match op {
            py::CmpOp::Eq => CmpOp::Eq,
            py::CmpOp::NotEq => CmpOp::NotEq,
            py::CmpOp::Lt => CmpOp::Lt,
            py::CmpOp::LtE => CmpOp::LtE,
            py::CmpOp::Gt => CmpOp::Gt,
            py::CmpOp::GtE => CmpOp::GtE,
            py::CmpOp::In => CmpOp::In,
            py::CmpOp::NotIn => CmpOp::NotIn,
            py::CmpOp::Is => CmpOp::Is,
            py::CmpOp::IsNot => CmpOp::IsNot,
        })
    }
}
