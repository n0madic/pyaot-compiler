//! Import extraction and resolution
//!
//! This module provides utilities for extracting and resolving Python imports
//! using AST-based parsing for accurate import detection.

use crate::types::ExtractedImport;
use miette::Result;
use rustpython_parser as rpy;

/// Return true when `name` (or its dotted root) resolves to a module we know
/// is provided by the compiler rather than by user-level `.py` files — either
/// a stdlib module registered in `pyaot-stdlib-defs` or a third-party package
/// registered in `pyaot-pkg-defs`. These must be skipped during module
/// discovery so the graph builder doesn't try to locate them on disk and emit
/// spurious "module not found" warnings.
fn is_builtin_module(name: &str) -> bool {
    let root = name.split('.').next().unwrap_or(name);
    // `typing` is used only for annotations and is erased by the frontend; it
    // isn't registered as a runtime stdlib module.
    root == "typing"
        || pyaot_stdlib_defs::is_stdlib_module(root)
        || pyaot_pkg_defs::is_package(root)
}

/// Extract imports from a source file using AST parsing.
/// Returns ExtractedImport structs with level and module path.
///
/// # Errors
/// Returns an error if the source file cannot be parsed (syntax error).
pub fn extract_imports_with_level(source: &str) -> Result<Vec<ExtractedImport>> {
    let ast = rpy::parse(source, rpy::Mode::Module, "<module>")
        .map_err(|e| miette::miette!("Parse error: {}", e.error))?;

    let mut imports = Vec::new();

    if let rpy::ast::Mod::Module(module) = ast {
        for stmt in &module.body {
            extract_imports_from_stmt(stmt, &mut imports);
        }
    }

    Ok(imports)
}

/// Recursively extract imports from a statement, handling nested structures.
fn extract_imports_from_stmt(stmt: &rpy::ast::Stmt, imports: &mut Vec<ExtractedImport>) {
    match stmt {
        rpy::ast::Stmt::Import(import_stmt) => {
            handle_import(import_stmt, imports);
        }
        rpy::ast::Stmt::ImportFrom(import_from) => {
            handle_import_from(import_from, imports);
        }
        // Handle nested statements that can contain imports
        rpy::ast::Stmt::If(if_stmt) => {
            for s in &if_stmt.body {
                extract_imports_from_stmt(s, imports);
            }
            for s in &if_stmt.orelse {
                extract_imports_from_stmt(s, imports);
            }
        }
        rpy::ast::Stmt::Try(try_stmt) => {
            for s in &try_stmt.body {
                extract_imports_from_stmt(s, imports);
            }
            for handler in &try_stmt.handlers {
                let rpy::ast::ExceptHandler::ExceptHandler(h) = handler;
                for s in &h.body {
                    extract_imports_from_stmt(s, imports);
                }
            }
            for s in &try_stmt.orelse {
                extract_imports_from_stmt(s, imports);
            }
            for s in &try_stmt.finalbody {
                extract_imports_from_stmt(s, imports);
            }
        }
        rpy::ast::Stmt::With(with_stmt) => {
            for s in &with_stmt.body {
                extract_imports_from_stmt(s, imports);
            }
        }
        rpy::ast::Stmt::FunctionDef(func) => {
            for s in &func.body {
                extract_imports_from_stmt(s, imports);
            }
        }
        rpy::ast::Stmt::AsyncFunctionDef(func) => {
            for s in &func.body {
                extract_imports_from_stmt(s, imports);
            }
        }
        rpy::ast::Stmt::ClassDef(class) => {
            for s in &class.body {
                extract_imports_from_stmt(s, imports);
            }
        }
        rpy::ast::Stmt::For(for_stmt) => {
            for s in &for_stmt.body {
                extract_imports_from_stmt(s, imports);
            }
            for s in &for_stmt.orelse {
                extract_imports_from_stmt(s, imports);
            }
        }
        rpy::ast::Stmt::AsyncFor(for_stmt) => {
            for s in &for_stmt.body {
                extract_imports_from_stmt(s, imports);
            }
            for s in &for_stmt.orelse {
                extract_imports_from_stmt(s, imports);
            }
        }
        rpy::ast::Stmt::While(while_stmt) => {
            for s in &while_stmt.body {
                extract_imports_from_stmt(s, imports);
            }
            for s in &while_stmt.orelse {
                extract_imports_from_stmt(s, imports);
            }
        }
        rpy::ast::Stmt::AsyncWith(with_stmt) => {
            for s in &with_stmt.body {
                extract_imports_from_stmt(s, imports);
            }
        }
        rpy::ast::Stmt::Match(match_stmt) => {
            for case in &match_stmt.cases {
                for s in &case.body {
                    extract_imports_from_stmt(s, imports);
                }
            }
        }
        // Other statement types don't contain nested imports
        _ => {}
    }
}

/// Handle `import module` and `import pkg.submodule` statements.
fn handle_import(import_stmt: &rpy::ast::StmtImport, imports: &mut Vec<ExtractedImport>) {
    for alias in &import_stmt.names {
        let module_name = alias.name.as_str();

        // Skip stdlib modules and registered packages — they're provided by
        // the compiler, not resolved from user `.py` files.
        if is_builtin_module(module_name) {
            continue;
        }

        imports.push(ExtractedImport {
            module_path: module_name.to_string(),
            level: 0,
        });
    }
}

/// Handle `from module import ...` and `from pkg.sub import ...` statements.
fn handle_import_from(import_from: &rpy::ast::StmtImportFrom, imports: &mut Vec<ExtractedImport>) {
    // Extract level from Option<Int> (None means 0, Some(Int(n)) means n dots)
    let level: u32 = import_from
        .level
        .as_ref()
        .map(|int| int.to_u32())
        .unwrap_or(0);

    let module_path = import_from
        .module
        .as_ref()
        .map(|id| id.as_str().to_string())
        .unwrap_or_default();

    // For absolute imports (level 0), skip stdlib + registered packages.
    if level == 0 && is_builtin_module(&module_path) {
        return;
    }

    // Add the main module import
    imports.push(ExtractedImport {
        module_path: module_path.clone(),
        level,
    });

    // Also add potential submodule imports
    // "from pkg import mod" might mean we need "pkg.mod" if "mod" is a submodule
    for alias in &import_from.names {
        let name = alias.name.as_str();

        // Skip wildcard imports and non-identifier names
        if name == "*" || !is_valid_identifier(name) {
            continue;
        }

        let submodule_path = if module_path.is_empty() {
            name.to_string()
        } else {
            format!("{}.{}", module_path, name)
        };

        imports.push(ExtractedImport {
            module_path: submodule_path,
            level,
        });
    }
}

/// Check if a string is a valid Python identifier.
fn is_valid_identifier(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| c.is_alphanumeric() || c == '_')
        && !s.chars().next().unwrap_or('0').is_numeric()
}

/// Resolve a relative import to an absolute module path.
///
/// # Arguments
/// * `importing_module_path` - Full dotted path of the importing module (e.g., "pkg.sub.mod")
/// * `relative_path` - Module path after dots (e.g., "utils" for "from .utils import")
/// * `level` - Number of dots (1 for ".", 2 for "..", etc.)
/// * `importing_is_package` - True if the importing module is an __init__.py
///
/// # Errors
/// Returns an error if:
/// - The relative import goes above the top-level package
/// - The importing module has no parent package (e.g., top-level script)
///
/// # PEP 328 Compliance
/// Relative imports require the importing module to be part of a package structure.
/// A module `foo.py` run as a script has no parent package and cannot use relative imports.
pub fn resolve_relative_import(
    importing_module_path: &str,
    relative_path: &str,
    level: u32,
    importing_is_package: bool,
) -> Result<String, String> {
    if level == 0 {
        return Ok(relative_path.to_string());
    }

    // Top-level scripts have no package context
    if importing_module_path.is_empty() {
        return Err("Relative import in top-level script: no parent package exists".to_string());
    }

    let mut parts: Vec<&str> = importing_module_path.split('.').collect();

    // For regular modules, go up one level to get the containing package
    // For __init__.py (package), the module path IS the package
    if !importing_is_package {
        parts.pop();
        // After popping, if parts is empty, this module has no parent package
        if parts.is_empty() {
            return Err(format!(
                "Relative import in module '{}' which has no parent package",
                importing_module_path
            ));
        }
    }

    // Go up (level - 1) more levels
    // Use >= to ensure we keep at least one package level
    let levels_up = (level - 1) as usize;
    if levels_up >= parts.len() {
        return Err(format!(
            "Relative import goes above top-level package: {} dots from '{}'",
            level, importing_module_path
        ));
    }
    for _ in 0..levels_up {
        parts.pop();
    }

    // Build absolute path - at this point base is guaranteed non-empty
    let base = parts.join(".");
    debug_assert!(
        !base.is_empty(),
        "Internal error: base should not be empty after level checks"
    );

    if relative_path.is_empty() {
        Ok(base)
    } else {
        Ok(format!("{}.{}", base, relative_path))
    }
}

/// Rewrite relative imports in source code to absolute imports.
///
/// This transforms lines like:
/// - `from .utils import func` -> `from pkg.utils import func`
/// - `from .. import name` -> `from parent import name`
/// - `from ..other import func` -> `from parent.other import func`
pub fn rewrite_relative_imports(
    source: &str,
    full_module_path: &str,
    is_package_init: bool,
) -> Result<String> {
    let mut lines: Vec<String> = Vec::new();

    for line in source.lines() {
        let trimmed = line.trim();

        // Check for `from .xxx import ...` pattern
        if trimmed.starts_with("from ") && trimmed.contains(" import ") {
            if let Some(module_part) = trimmed.strip_prefix("from ") {
                if let Some(idx) = module_part.find(" import ") {
                    let raw_module = &module_part[..idx];
                    let import_part = &module_part[idx..]; // " import ..."

                    // Count leading dots for relative import level
                    let mut level: u32 = 0;
                    let mut chars = raw_module.chars().peekable();
                    while chars.peek() == Some(&'.') {
                        level += 1;
                        chars.next();
                    }

                    if level > 0 {
                        // This is a relative import - resolve it
                        let relative_path: String = chars.collect();
                        let relative_path = relative_path.trim();

                        match resolve_relative_import(
                            full_module_path,
                            relative_path,
                            level,
                            is_package_init,
                        ) {
                            Ok(absolute_path) => {
                                // Preserve leading whitespace from original line
                                let leading_ws: String =
                                    line.chars().take_while(|c| c.is_whitespace()).collect();
                                let rewritten =
                                    format!("{}from {}{}", leading_ws, absolute_path, import_part);
                                lines.push(rewritten);
                                continue;
                            }
                            Err(e) => {
                                return Err(miette::miette!(
                                    "Failed to resolve relative import in '{}': {}",
                                    full_module_path,
                                    e
                                ));
                            }
                        }
                    }
                }
            }
        }

        // Non-import line or absolute import - keep as-is
        lines.push(line.to_string());
    }

    Ok(lines.join("\n"))
}

#[cfg(test)]
#[path = "import_resolver_tests.rs"]
mod import_resolver_tests;
