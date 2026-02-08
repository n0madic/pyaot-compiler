//! Binary and unary operators for MIR

/// Binary operations in MIR
#[derive(Debug, Clone, Copy)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    FloorDiv,
    Mod,
    Pow,
    Eq,
    NotEq,
    Lt,
    LtE,
    Gt,
    GtE,
    And,
    Or,
    // Bitwise operators
    BitAnd,
    BitOr,
    BitXor,
    LShift,
    RShift,
}

/// Unary operations in MIR
#[derive(Debug, Clone, Copy)]
pub enum UnOp {
    Neg,
    Not,
    Invert, // Bitwise NOT (~)
}
