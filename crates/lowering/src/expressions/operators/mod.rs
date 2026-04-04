//! Operator expression lowering: BinOp, Compare, UnOp, LogicalOp
//!
//! Split into focused submodules:
//! - `binary_ops`: Arithmetic, bitwise, string, and collection binary operations
//! - `comparison`: Equality, ordering, identity, and containment checks
//! - `unary_ops`: Negation, boolean not, bitwise invert, unary plus
//! - `logical_ops`: Short-circuit `and`/`or` and ternary `if` expressions

mod binary_ops;
mod comparison;
mod logical_ops;
mod unary_ops;
