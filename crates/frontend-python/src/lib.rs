//! Python frontend: parsing and AST to HIR conversion

#![forbid(unsafe_code)]

pub mod ast_to_hir;
pub mod parser;

pub use ast_to_hir::AstToHir;
pub use parser::parse_module;
