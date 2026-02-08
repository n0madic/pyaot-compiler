//! Access expression lowering: Slice, Index, MethodCall, Attribute, Super
//!
//! This module provides entry points for lowering various access expressions from HIR to MIR.
//! The implementation is split into focused submodules for better maintainability:
//!
//! - `slicing`: Slice operations (obj[start:end:step])
//! - `indexing`: Index operations (obj[index])
//! - `attributes`: Attribute access (obj.attr) and super calls (super().method())
//! - `method/`: Method call implementations (organized by type)
//!   - `method/dispatch`: Main dispatcher that routes to type-specific handlers
//!   - `method/string`, `method/list`, `method/dict`, etc.: Type-specific implementations

// Module declarations
mod attributes;
mod indexing;
mod method;
mod slicing;

// Re-export public entry points (visible to parent expressions/ module)
// All methods are already implemented as `impl Lowering` in their respective files
