//! Core type definitions for the CLI

use pyaot_hir as hir;
use pyaot_utils::StringInterner;
use std::path::PathBuf;

/// Information about a parsed module
pub struct ParsedModule {
    /// Filesystem path to the source file
    pub path: PathBuf,
    pub source: String,
    pub hir: hir::Module,
    pub interner: StringInterner,
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
