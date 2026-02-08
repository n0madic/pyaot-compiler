//! Core type definitions for the CLI

use pyaot_hir as hir;
use pyaot_utils::StringInterner;
use std::path::PathBuf;

// Re-export from lowering to avoid duplicate definitions
pub use pyaot_lowering::CrossModuleClassInfo;

/// Information about a parsed module
pub struct ParsedModule {
    /// Simple module name (e.g., "utils" or "__init__")
    /// Stored for debugging and potential future logging
    #[allow(dead_code)]
    pub name: String,
    /// Full dotted module path (e.g., "pkg.submodule" or "pkg")
    /// Stored for debugging and potential future logging
    #[allow(dead_code)]
    pub full_module_path: String,
    /// Filesystem path to the source file
    pub path: PathBuf,
    pub source: String,
    pub hir: hir::Module,
    pub interner: StringInterner,
    /// True if this is a package __init__.py
    /// Stored for debugging and potential future use
    #[allow(dead_code)]
    pub is_package_init: bool,
    /// Parent package name (e.g., "pkg" for "pkg.submodule")
    pub parent_package: Option<String>,
}

/// Extracted import information including relative import level
#[derive(Debug, Clone)]
pub struct ExtractedImport {
    /// Module path after dots (e.g., "utils" for "from .utils import")
    pub module_path: String,
    /// 0=absolute, 1=from ., 2=from .., etc.
    pub level: u32,
}

/// Result of module resolution
pub enum ModuleResolution {
    /// A simple .py file
    File(PathBuf),
    /// A package directory with __init__.py
    Package { init_path: PathBuf },
    /// A submodule within a package (includes parent __init__.py files)
    Submodule {
        module_path: PathBuf,
        /// List of parent __init__.py files that need to be loaded first
        package_inits: Vec<(String, PathBuf)>,
    },
}
