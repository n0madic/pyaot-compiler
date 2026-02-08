//! Binary and unary operation type inference

use pyaot_hir::{BinOp, UnOp};
use pyaot_types::Type;

use crate::context::TypeChecker;

impl<'a> TypeChecker<'a> {
    /// Infer type of binary operation
    pub(crate) fn infer_binop_type(&self, op: BinOp, left: &Type, right: &Type) -> Type {
        // For class types with arithmetic dunders, return the left operand type
        // (Python convention: __add__ etc. return the same class type)
        if matches!(left, Type::Class { .. }) {
            return left.clone();
        }

        match op {
            BinOp::Add => {
                // String concatenation
                if left == &Type::Str && right == &Type::Str {
                    return Type::Str;
                }
                // String multiplication
                if (left == &Type::Str && right == &Type::Int)
                    || (left == &Type::Int && right == &Type::Str)
                {
                    return Type::Str;
                }
                // Numeric operations
                if left == &Type::Float || right == &Type::Float {
                    Type::Float
                } else if left == &Type::Int && right == &Type::Int {
                    Type::Int
                } else {
                    Type::Any
                }
            }
            BinOp::Sub | BinOp::Mul | BinOp::Pow => {
                // String multiplication
                if op == BinOp::Mul
                    && ((left == &Type::Str && right == &Type::Int)
                        || (left == &Type::Int && right == &Type::Str))
                {
                    return Type::Str;
                }
                // Numeric operations
                if left == &Type::Float || right == &Type::Float {
                    Type::Float
                } else if left == &Type::Int && right == &Type::Int {
                    Type::Int
                } else {
                    Type::Any
                }
            }
            BinOp::Div => {
                // Division always returns float in Python 3
                Type::Float
            }
            BinOp::FloorDiv | BinOp::Mod => {
                if left == &Type::Float || right == &Type::Float {
                    Type::Float
                } else {
                    Type::Int
                }
            }
            // Bitwise operators always return Int
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::LShift | BinOp::RShift => {
                Type::Int
            }
        }
    }

    /// Infer type of unary operation
    pub(crate) fn infer_unop_type(&self, op: UnOp, operand: &Type) -> Type {
        match op {
            UnOp::Neg => {
                if operand == &Type::Float {
                    Type::Float
                } else if operand == &Type::Int {
                    Type::Int
                } else {
                    Type::Any
                }
            }
            UnOp::Not => Type::Bool,
            UnOp::Invert => Type::Int, // Bitwise NOT always returns Int
        }
    }
}
