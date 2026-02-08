//! Import statements: ImportFrom, Import
//!
//! This module handles conversion of Python import statements to HIR.
//! Stdlib imports are validated against the pyaot-stdlib-defs registry.

use super::AstToHir;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::*;
use pyaot_stdlib_defs::{self as stdlib, StdlibItem as RegistryItem};
use pyaot_utils::Span;
use rustpython_parser::ast as py;

impl AstToHir {
    pub(crate) fn convert_import_from(
        &mut self,
        import_from: py::StmtImportFrom,
        stmt_span: Span,
    ) -> Result<StmtId> {
        // Handle `from typing import ...` - store imported names for type annotation resolution
        if import_from.module.as_deref() == Some("typing") {
            for alias in &import_from.names {
                let name = self.interner.intern(&alias.name);
                self.typing_imports.insert(name);
            }
            // Return Pass statement (import is compile-time only)
            return Ok(self.module.stmts.alloc(Stmt {
                kind: StmtKind::Pass,
                span: stmt_span,
            }));
        }

        // Check if this is a stdlib module
        let module_name = import_from.module.as_deref().unwrap_or("");
        if let Some(module_def) = stdlib::get_module(module_name) {
            // Handle stdlib import
            for alias in &import_from.names {
                let local_name = if let Some(ref asname) = alias.asname {
                    self.interner.intern(asname.as_str())
                } else {
                    self.interner.intern(&alias.name)
                };

                // Look up the name in the module definition
                if let Some(item) = stdlib::get_item(module_def.name, &alias.name) {
                    match item {
                        RegistryItem::Function(func_def) => {
                            // Store reference to definition (Single Source of Truth)
                            self.stdlib_names
                                .insert(local_name, super::super::StdlibItem::Func(func_def));
                        }
                        RegistryItem::Attr(attr_def) => {
                            // Store reference to definition (Single Source of Truth)
                            self.stdlib_names
                                .insert(local_name, super::super::StdlibItem::Attr(attr_def));
                        }
                        RegistryItem::Constant(const_def) => {
                            // Store reference to definition (Single Source of Truth)
                            // Constants are inlined at compile time
                            self.stdlib_names
                                .insert(local_name, super::super::StdlibItem::Const(const_def));
                        }
                        RegistryItem::Class(_class_def) => {
                            // Classes like re.Match are not typically imported directly
                            return Err(CompilerError::parse_error(
                                format!(
                                    "Stdlib class '{}' from module '{}' cannot be directly imported",
                                    alias.name, module_name
                                ),
                                stmt_span,
                            ));
                        }
                    }
                } else {
                    // Name not found in module definition
                    let available = stdlib::list_all_names(module_name);
                    return Err(CompilerError::parse_error(
                        format!(
                            "Unknown attribute '{}' in module '{}'. Available: {}",
                            alias.name,
                            module_name,
                            available.join(", ")
                        ),
                        stmt_span,
                    ));
                }
            }

            // Return Pass statement (import is compile-time only)
            return Ok(self.module.stmts.alloc(Stmt {
                kind: StmtKind::Pass,
                span: stmt_span,
            }));
        }

        // Handle user module imports (non-stdlib)
        let module_name = import_from.module.as_deref().unwrap_or("").to_string();

        // Record the import declaration
        let mut names = Vec::new();
        for alias in &import_from.names {
            let original_name = self.interner.intern(&alias.name);
            let local_name = if let Some(ref asname) = alias.asname {
                self.interner.intern(asname.as_str())
            } else {
                original_name
            };
            names.push((
                original_name,
                alias
                    .asname
                    .as_ref()
                    .map(|n| self.interner.intern(n.as_str())),
            ));

            // Record the imported name for expression resolution
            self.imported_names.insert(
                local_name,
                super::super::ImportedName {
                    module: module_name.clone(),
                    original_name: alias.name.to_string(),
                    kind: super::super::ImportedNameKind::Unresolved,
                },
            );
        }

        // Add import declaration to module
        self.module.imports.push(pyaot_hir::ImportDecl {
            module_path: module_name,
            kind: pyaot_hir::ImportKind::FromImport { names },
            is_package: false, // Will be set by CLI during module discovery
            span: stmt_span,
        });

        // Return Pass statement (import is compile-time only, actual resolution
        // happens during multi-module merging)
        Ok(self.module.stmts.alloc(Stmt {
            kind: StmtKind::Pass,
            span: stmt_span,
        }))
    }

    pub(crate) fn convert_import(
        &mut self,
        import_stmt: py::StmtImport,
        stmt_span: Span,
    ) -> Result<StmtId> {
        // Handle `import module` or `import module as alias` or `import pkg.submodule`
        for alias in &import_stmt.names {
            let module_name = alias.name.to_string();

            // For dotted imports like `import pkg.submodule`, the local name is the first part
            // unless an alias is provided. E.g., `import pkg.sub` binds to `pkg`
            let local_name = if let Some(ref asname) = alias.asname {
                self.interner.intern(asname.as_str())
            } else if module_name.contains('.') {
                // For `import pkg.sub.module`, local name is `pkg`
                let first_part = module_name.split('.').next().unwrap_or(&module_name);
                self.interner.intern(first_part)
            } else {
                self.interner.intern(&alias.name)
            };

            // Check if this is a stdlib module using the registry
            let root_module = stdlib::get_root_module(&module_name);
            if stdlib::is_stdlib_module(root_module) {
                // Record as stdlib import for expression handling
                self.stdlib_imports.insert(local_name);
            } else {
                // Record the imported module for attribute access
                // For `import pkg.sub`, we map `pkg` -> "pkg" (the root only)
                // Submodule access like pkg.sub.func() will be handled via chained attr
                if module_name.contains('.') {
                    // For dotted imports, record the root package
                    let root = module_name.split('.').next().unwrap_or(&module_name);
                    self.imported_modules.insert(local_name, root.to_string());

                    // Also record the full dotted path for chained access resolution
                    self.dotted_imports
                        .insert(module_name.clone(), module_name.clone());
                } else {
                    self.imported_modules
                        .insert(local_name, module_name.clone());
                }

                // Add import declaration to module
                self.module.imports.push(pyaot_hir::ImportDecl {
                    module_path: module_name,
                    kind: pyaot_hir::ImportKind::Module {
                        alias: alias
                            .asname
                            .as_ref()
                            .map(|n| self.interner.intern(n.as_str())),
                    },
                    is_package: false, // Will be set by CLI during module discovery
                    span: stmt_span,
                });
            }
        }

        // Return Pass statement (import is compile-time only)
        Ok(self.module.stmts.alloc(Stmt {
            kind: StmtKind::Pass,
            span: stmt_span,
        }))
    }
}
