//! Import statements: ImportFrom, Import
//!
//! This module handles conversion of Python import statements to HIR.
//! Stdlib imports are validated against the pyaot-stdlib-defs registry.

use super::AstToHir;
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_hir::*;
use pyaot_pkg_defs as pkgs;
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
                self.types.typing_imports.insert(name);
            }
            // Return Pass statement (import is compile-time only)
            return Ok(self.module.stmts.alloc(Stmt {
                kind: StmtKind::Pass,
                span: stmt_span,
            }));
        }

        // Check if this is a stdlib or a registered third-party package.
        // Both registries expose the same `StdlibModuleDef` shape (via type
        // alias in `pyaot_pkg_defs`), so the handling below is identical —
        // we only need to additionally record the package name so the CLI
        // can link its `.a` archive selectively.
        let module_name = import_from.module.as_deref().unwrap_or("");
        let module_def = stdlib::get_module(module_name).or_else(|| {
            pkgs::get_package(module_name).inspect(|_| {
                self.module
                    .used_packages
                    .insert(pkgs::get_root_package(module_name).to_string());
            })
        });
        if let Some(module_def) = module_def {
            // Handle stdlib / package import
            for alias in &import_from.names {
                let local_name = if let Some(ref asname) = alias.asname {
                    self.interner.intern(asname.as_str())
                } else {
                    self.interner.intern(&alias.name)
                };

                // Look up the name in the module definition. We use the
                // `get_*` methods on the module struct directly so the
                // lookup works uniformly for stdlib and package modules
                // without depending on the stdlib `ALL_MODULES` registry.
                let item = if let Some(f) = module_def.get_function(&alias.name) {
                    Some(RegistryItem::Function(f))
                } else if let Some(a) = module_def.get_attr(&alias.name) {
                    Some(RegistryItem::Attr(a))
                } else if let Some(c) = module_def.get_constant(&alias.name) {
                    Some(RegistryItem::Constant(c))
                } else {
                    module_def.get_class(&alias.name).map(RegistryItem::Class)
                };

                if let Some(item) = item {
                    match item {
                        RegistryItem::Function(func_def) => {
                            // Store reference to definition (Single Source of Truth)
                            self.imports
                                .stdlib_names
                                .insert(local_name, super::super::StdlibItem::Func(func_def));
                        }
                        RegistryItem::Attr(attr_def) => {
                            // Store reference to definition (Single Source of Truth)
                            self.imports
                                .stdlib_names
                                .insert(local_name, super::super::StdlibItem::Attr(attr_def));
                        }
                        RegistryItem::Constant(const_def) => {
                            // Store reference to definition (Single Source of Truth)
                            // Constants are inlined at compile time
                            self.imports
                                .stdlib_names
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
                    let mut available: Vec<&str> = Vec::new();
                    available.extend(module_def.functions.iter().map(|f| f.name));
                    available.extend(module_def.attrs.iter().map(|a| a.name));
                    available.extend(module_def.constants.iter().map(|c| c.name));
                    available.extend(module_def.classes.iter().map(|c| c.name));
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
            self.imports.imported_names.insert(
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

            // Check if this is a stdlib module or a registered third-party
            // package. Packages reuse the stdlib lowering path (attribute
            // access resolves through `StdlibModuleDef` either way), so we
            // fold them into `stdlib_imports` and additionally record the
            // root name for selective linking.
            let root_module = stdlib::get_root_module(&module_name);
            let is_stdlib = stdlib::is_stdlib_module(root_module);
            let is_pkg = !is_stdlib && pkgs::is_package(root_module);
            if is_stdlib || is_pkg {
                if is_pkg {
                    self.module.used_packages.insert(root_module.to_string());
                }
                // Record as stdlib-style import for expression handling
                self.imports.stdlib_imports.insert(local_name);
            } else {
                // Record the imported module for attribute access
                // For `import pkg.sub`, we map `pkg` -> "pkg" (the root only)
                // Submodule access like pkg.sub.func() will be handled via chained attr
                if module_name.contains('.') {
                    // For dotted imports, record the root package
                    let root = module_name.split('.').next().unwrap_or(&module_name);
                    self.imports
                        .imported_modules
                        .insert(local_name, root.to_string());

                    // Also record the full dotted path for chained access resolution
                    self.imports
                        .dotted_imports
                        .insert(module_name.clone(), module_name.clone());
                } else {
                    self.imports
                        .imported_modules
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
