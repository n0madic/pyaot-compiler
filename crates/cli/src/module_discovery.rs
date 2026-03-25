//! Module discovery and loading

use crate::import_resolver::{
    extract_imports_with_level, resolve_relative_import, rewrite_relative_imports,
};
use crate::types::{ModuleResolution, ParsedModule};
use miette::{IntoDiagnostic, NamedSource, Report, Result};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

pub struct ModuleDiscovery {
    /// Directories to search for modules
    search_paths: Vec<PathBuf>,
    /// Parsed modules by name
    parsed_modules: HashMap<String, ParsedModule>,
    /// Module dependency graph: module -> modules it imports
    dependencies: HashMap<String, Vec<String>>,
    /// Set of modules currently being parsed (for cycle detection)
    parsing_stack: Vec<String>,
    /// Verbose output
    verbose: bool,
}

impl ModuleDiscovery {
    pub fn new(search_paths: Vec<PathBuf>, verbose: bool) -> Self {
        Self {
            search_paths,
            parsed_modules: HashMap::new(),
            dependencies: HashMap::new(),
            parsing_stack: Vec::new(),
            verbose,
        }
    }

    /// Find a module or package by name, supporting dotted paths like "pkg.submodule"
    fn find_module_or_package(
        &self,
        module_name: &str,
        relative_to: &Path,
    ) -> Option<ModuleResolution> {
        let parts: Vec<&str> = module_name.split('.').collect();

        // Try each search path (including the directory relative to importing module)
        let mut search_dirs = Vec::new();
        if let Some(parent) = relative_to.parent() {
            search_dirs.push(parent.to_path_buf());
        }
        search_dirs.extend(self.search_paths.clone());

        for base_dir in &search_dirs {
            if let Some(resolution) = self.resolve_module_in_dir(base_dir, &parts) {
                return Some(resolution);
            }
        }

        None
    }

    /// Resolve a module path within a specific directory
    fn resolve_module_in_dir(&self, base_dir: &Path, parts: &[&str]) -> Option<ModuleResolution> {
        if parts.is_empty() {
            return None;
        }

        if parts.len() == 1 {
            // Simple name: check for package first, then file
            let name = parts[0];

            // Check for package: name/__init__.py
            let init_path = base_dir.join(name).join("__init__.py");
            if init_path.exists() {
                return Some(ModuleResolution::Package { init_path });
            }

            // Check for file: name.py
            let file_path = base_dir.join(format!("{}.py", name));
            if file_path.exists() {
                return Some(ModuleResolution::File(file_path));
            }

            None
        } else {
            // Dotted path: traverse directories and collect __init__.py files
            let mut current_dir = base_dir.to_path_buf();
            let mut package_inits = Vec::new();
            let mut accumulated_path = String::new();

            // Process all but the last part as package directories
            for (i, part) in parts.iter().enumerate() {
                if i < parts.len() - 1 {
                    // This is a package segment
                    current_dir = current_dir.join(part);
                    let init_path = current_dir.join("__init__.py");

                    if !init_path.exists() {
                        // Not a valid package path
                        return None;
                    }

                    // Build accumulated module path
                    if accumulated_path.is_empty() {
                        accumulated_path = (*part).to_string();
                    } else {
                        accumulated_path = format!("{}.{}", accumulated_path, part);
                    }

                    package_inits.push((accumulated_path.clone(), init_path));
                } else {
                    // Last segment: can be either submodule.py or subpackage/__init__.py
                    // Check for subpackage first (Python prefers packages over files)
                    let subpackage_dir = current_dir.join(part);
                    let subpackage_init = subpackage_dir.join("__init__.py");
                    if subpackage_init.exists() {
                        return Some(ModuleResolution::Submodule {
                            module_path: subpackage_init,
                            package_inits,
                        });
                    }

                    // Check for submodule file
                    let module_file = current_dir.join(format!("{}.py", part));
                    if module_file.exists() {
                        return Some(ModuleResolution::Submodule {
                            module_path: module_file,
                            package_inits,
                        });
                    }

                    return None;
                }
            }

            None
        }
    }

    /// Recursively discover and parse all required modules (including packages)
    pub fn discover_modules(&mut self, module_name: &str, module_path: &PathBuf) -> Result<()> {
        self.discover_modules_with_context(
            module_name,
            module_path,
            module_name.to_string(),
            false,
            None,
        )
    }

    /// Internal helper for discovering modules with full context
    fn discover_modules_with_context(
        &mut self,
        module_name: &str,
        module_path: &PathBuf,
        full_module_path: String,
        is_package_init: bool,
        parent_package: Option<String>,
    ) -> Result<()> {
        // Check for circular imports using full module path
        if self.parsing_stack.contains(&full_module_path) {
            let cycle = self.parsing_stack.join(" -> ");
            return Err(miette::miette!(
                "Circular import detected: {} -> {}",
                cycle,
                full_module_path
            ));
        }

        // Skip if already parsed
        if self.parsed_modules.contains_key(&full_module_path) {
            return Ok(());
        }

        self.parsing_stack.push(full_module_path.clone());

        // Read and parse the module
        let source = fs::read_to_string(module_path).into_diagnostic()?;

        // Extract imports first (with level info for relative imports)
        let extracted_imports = extract_imports_with_level(&source)?;

        // Resolve relative imports to absolute paths
        let mut resolved_imports = Vec::new();
        for import in &extracted_imports {
            if import.level > 0 {
                // Relative import - resolve to absolute path
                match resolve_relative_import(
                    &full_module_path,
                    &import.module_path,
                    import.level,
                    is_package_init,
                ) {
                    Ok(absolute) => {
                        if self.verbose {
                            println!(
                                "  Resolved relative import: {} dots + '{}' -> '{}'",
                                import.level, import.module_path, absolute
                            );
                        }
                        resolved_imports.push(absolute);
                    }
                    Err(e) => {
                        return Err(miette::miette!(
                            "Failed to resolve relative import in '{}': {}",
                            full_module_path,
                            e
                        ));
                    }
                }
            } else {
                // Absolute import
                resolved_imports.push(import.module_path.clone());
            }
        }

        // Build dependencies: include parent package if present
        let mut deps = Vec::new();
        if let Some(ref parent) = parent_package {
            deps.push(parent.clone());
        }
        deps.extend(resolved_imports.clone());
        self.dependencies.insert(full_module_path.clone(), deps);

        // Recursively discover imported modules
        for import_name in &resolved_imports {
            if let Some(resolution) = self.find_module_or_package(import_name, module_path) {
                match resolution {
                    ModuleResolution::File(path) => {
                        self.discover_modules_with_context(
                            import_name,
                            &path,
                            import_name.clone(),
                            false,
                            None,
                        )?;
                    }
                    ModuleResolution::Package { init_path } => {
                        self.discover_modules_with_context(
                            import_name,
                            &init_path,
                            import_name.clone(),
                            true,
                            None,
                        )?;
                    }
                    ModuleResolution::Submodule {
                        module_path: sub_path,
                        package_inits,
                    } => {
                        // First, ensure all parent packages are discovered
                        // Skip packages that are already parsed OR currently being parsed
                        let mut prev_package: Option<String> = None;
                        for (pkg_path, init_path) in &package_inits {
                            if !self.parsed_modules.contains_key(pkg_path)
                                && !self.parsing_stack.contains(pkg_path)
                            {
                                self.discover_modules_with_context(
                                    pkg_path.split('.').next_back().unwrap_or(pkg_path),
                                    init_path,
                                    pkg_path.clone(),
                                    true,
                                    prev_package.clone(),
                                )?;
                            }
                            prev_package = Some(pkg_path.clone());
                        }

                        // Then discover the submodule itself
                        let is_subpackage = sub_path
                            .file_name()
                            .map(|n| n == "__init__.py")
                            .unwrap_or(false);
                        self.discover_modules_with_context(
                            import_name.split('.').next_back().unwrap_or(import_name),
                            &sub_path,
                            import_name.clone(),
                            is_subpackage,
                            prev_package,
                        )?;
                    }
                }
            } else if self.verbose {
                eprintln!(
                    "Warning: Could not find module '{}' imported by '{}'",
                    import_name, full_module_path
                );
            }
        }

        // Rewrite relative imports to absolute imports before parsing
        let source = rewrite_relative_imports(&source, &full_module_path, is_package_init)?;

        // Parse the module
        if self.verbose {
            println!(
                "Parsing module: {} (full path: {}, is_package: {})",
                module_name, full_module_path, is_package_init
            );
        }

        let ast = pyaot_frontend_python::parse_module(&source).map_err(|e| {
            Report::new(e).with_source_code(NamedSource::new(
                module_path.display().to_string(),
                source.clone(),
            ))
        })?;

        // Use the full module path for name mangling
        let ast_to_hir = pyaot_frontend_python::ast_to_hir::AstToHir::new(&full_module_path);
        let (hir_module, interner) = ast_to_hir.convert(ast).map_err(|e| {
            Report::new(e).with_source_code(NamedSource::new(
                module_path.display().to_string(),
                source.clone(),
            ))
        })?;

        self.parsed_modules.insert(
            full_module_path.clone(),
            ParsedModule {
                path: module_path.clone(),
                source,
                hir: hir_module,
                interner,
                parent_package,
            },
        );

        self.parsing_stack.pop();
        Ok(())
    }

    /// Topological sort of modules (dependencies first)
    /// Ensures parent packages come before their submodules
    pub fn topological_sort(&self, main_module: &str) -> Vec<String> {
        let mut sorted = Vec::new();
        let mut visited = HashSet::new();
        let mut temp_visited = HashSet::new();

        fn visit(
            module: &str,
            dependencies: &HashMap<String, Vec<String>>,
            parsed_modules: &HashMap<String, ParsedModule>,
            visited: &mut HashSet<String>,
            temp_visited: &mut HashSet<String>,
            sorted: &mut Vec<String>,
        ) {
            if visited.contains(module) {
                return;
            }
            if temp_visited.contains(module) {
                return; // Cycle detected, but we already checked earlier
            }

            temp_visited.insert(module.to_string());

            // First visit the parent package if present
            if let Some(parsed) = parsed_modules.get(module) {
                if let Some(ref parent) = parsed.parent_package {
                    if parsed_modules.contains_key(parent) {
                        visit(
                            parent,
                            dependencies,
                            parsed_modules,
                            visited,
                            temp_visited,
                            sorted,
                        );
                    }
                }
            }

            // Then visit explicit dependencies
            if let Some(deps) = dependencies.get(module) {
                for dep in deps {
                    if parsed_modules.contains_key(dep) {
                        visit(
                            dep,
                            dependencies,
                            parsed_modules,
                            visited,
                            temp_visited,
                            sorted,
                        );
                    }
                }
            }

            temp_visited.remove(module);
            visited.insert(module.to_string());
            sorted.push(module.to_string());
        }

        visit(
            main_module,
            &self.dependencies,
            &self.parsed_modules,
            &mut visited,
            &mut temp_visited,
            &mut sorted,
        );

        sorted
    }

    pub fn take_modules(self) -> HashMap<String, ParsedModule> {
        self.parsed_modules
    }
}
