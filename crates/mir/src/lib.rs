//! Mid-level Intermediate Representation (MIR)
//!
//! This is a CFG-based SSA-like representation suitable for codegen.

#![forbid(unsafe_code)]

mod core;
mod instructions;
mod kinds;
mod operands;
mod operators;
mod runtime_func;
mod terminators;

// Re-export all public types
pub use core::{BasicBlock, Function, Local, Module, VtableEntry, VtableInfo};
pub use instructions::{Instruction, InstructionKind};
pub use kinds::{
    CompareKind, ComparisonOp, ContainerKind, ConversionTypeKind, ElementKind, IterDirection,
    IterSourceKind, MinMaxOp, PrintKind, ReprTargetKind, SortableKind, StringFormat, ValueKind,
};
pub use operands::{Constant, Operand};
pub use operators::{BinOp, UnOp};
pub use runtime_func::RuntimeFunc;
pub use terminators::{RaiseCause, Terminator};

// Re-export BuiltinFunctionKind for first-class builtin support
pub use pyaot_core_defs::BuiltinFunctionKind;
