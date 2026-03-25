//! Standard library definitions for the Python AOT compiler
//!
//! This crate provides a declarative, single source of truth for all supported
//! stdlib modules. It defines:
//!
//! - Module structures (functions, attributes, constants, classes)
//! - Type specifications for parameters and return values
//! - Runtime function names for code generation
//! - Compile-time constant values
//!
//! ## Design Goals
//!
//! 1. **Single Source of Truth**: All stdlib information is defined here
//! 2. **Compile-time Safety**: Errors in definitions are caught at compile time
//! 3. **Zero Dependencies**: This is a leaf crate with no pyaot-* dependencies
//! 4. **Declarative**: Modules are described, not implemented
//!
//! ## Usage
//!
//! ```text
//! use pyaot_stdlib_defs::{get_module, get_function, is_stdlib_module};
//!
//! // Check if a module is stdlib
//! if is_stdlib_module("sys") {
//!     let module = get_module("sys").unwrap();
//!
//!     // Get function definition
//!     if let Some(func) = module.get_function("exit") {
//!         println!("Runtime name: {}", func.runtime_name);
//!     }
//! }
//! ```
//!
//! ## Adding New Modules
//!
//! 1. Create a new file in `src/modules/` (e.g., `math.rs`)
//! 2. Define functions, attrs, constants using the types from `types.rs`
//! 3. Create the `StdlibModuleDef` static
//! 4. Add the module to `modules/mod.rs` in `ALL_MODULES`

#![forbid(unsafe_code)]

pub mod modules;
pub mod object_types;
pub mod registry;
pub mod types;

// Re-export commonly used items
pub use registry::{
    get_attr, get_class, get_constant, get_function, get_item, get_module, get_root_module,
    is_stdlib_module, list_all_names, list_attrs, list_classes, list_constants, list_functions,
    StdlibItem, StdlibItemKind,
};

pub use types::{
    ConstValue, LoweringHints, ParamDef, StdlibAttrDef, StdlibClassDef, StdlibConstDef,
    StdlibFunctionDef, StdlibMethodDef, StdlibModuleDef, TypeSpec, TypeSpecRef,
};

pub use object_types::{
    lookup_object_field, lookup_object_method, lookup_object_type, lookup_object_type_by_name,
    DisplayFormat, ObjectFieldDef, ObjectTypeDef, ALL_OBJECT_TYPES, COMPLETED_PROCESS, FILE, MATCH,
    STRUCT_TIME,
};

// Re-export module definitions for direct access
pub use modules::{json, math, os, re, sys};
