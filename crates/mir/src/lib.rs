//! Mid-level Intermediate Representation (MIR)
//!
//! This is a CFG-based SSA-like representation suitable for codegen.

#![forbid(unsafe_code)]

mod core;
pub mod dom_tree;
mod instructions;
mod kinds;
mod operands;
mod operators;
pub mod phi_normalize;
mod runtime_func;
pub mod ssa_check;
pub mod ssa_construct;
mod terminators;
pub mod types;
pub mod verify;

// Re-export all public types
pub use core::{
    BasicBlock, ClassMetadata, Function, FunctionKind, Local, Module, VtableEntry, VtableInfo,
};
pub use dom_tree::{terminator_successors, DomTree};
pub use instructions::{Instruction, InstructionKind};
pub use kinds::{
    CompareKind, ComparisonOp, ContainerKind, ConversionTypeKind, ElementKind, GetElementKind,
    IterDirection, IterSourceKind, MinMaxOp, PrintKind, ReprTargetKind, SearchOp, SortableKind,
    StringFormat,
};
pub use operands::{Constant, Operand};
pub use operators::{BinOp, UnOp};
pub use runtime_func::RuntimeFunc;
pub use terminators::{RaiseCause, Terminator};
pub use types::{
    type_to_mir_type_register, type_to_mir_type_storage, ClosureShape, HeapShape, MirType, RawKind,
    Signature,
};
pub use verify::{report_warnings, verify_function, verify_mir, MirError};

// Re-export BuiltinFunctionKind for first-class builtin support
pub use pyaot_core_defs::BuiltinFunctionKind;
