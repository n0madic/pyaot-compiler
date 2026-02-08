//! MIR operands and constants

use pyaot_utils::{InternedString, LocalId};

/// Operand (value used in instructions)
#[derive(Debug, Clone)]
pub enum Operand {
    Local(LocalId),
    Constant(Constant),
}

/// Constant value
#[derive(Debug, Clone)]
pub enum Constant {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(InternedString),
    Bytes(Vec<u8>),
    None,
}

impl Operand {
    pub fn local(id: LocalId) -> Self {
        Operand::Local(id)
    }

    pub fn constant(c: Constant) -> Self {
        Operand::Constant(c)
    }
}
