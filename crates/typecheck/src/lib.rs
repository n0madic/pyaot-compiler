//! Type checking for HIR
//!
//! This module performs static type checking on the HIR:
//! - Function call argument count and types
//! - Return type validation
//! - Binary/unary operation type compatibility
//! - Assignment type compatibility
//! - Method call validation

#![forbid(unsafe_code)]

mod builtins;
mod calls;
mod context;
mod expressions;
mod methods;
mod operators;
mod statements;
mod unpacking;

pub use context::TypeChecker;

#[cfg(test)]
mod tests;
