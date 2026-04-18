//! Python AOT Compiler library
//!
//! Provides the compilation pipeline as a library API, used by both the CLI
//! binary and integration tests.

#![forbid(unsafe_code)]

pub mod import_resolver;
pub mod mir_merger;
pub mod module_discovery;
pub mod pipeline;
pub mod types;

use miette::{IntoDiagnostic, Result};
use std::fs;
use std::path::{Path, PathBuf};
use target_lexicon::Triple;

/// Options for compiling a Python file to a native executable.
pub struct CompileOptions {
    /// Input Python source file
    pub input: PathBuf,
    /// Output executable path
    pub output: PathBuf,
    /// Path to the runtime library (libpyaot_runtime.a)
    pub runtime_lib: PathBuf,
    /// Additional directories to search for imported modules
    pub module_paths: Vec<PathBuf>,
    /// Enable function inlining optimization
    pub inline: bool,
    /// Maximum instruction count for inlining
    pub inline_threshold: usize,
    /// Enable dead code elimination optimization
    pub dce: bool,
    /// Enable constant folding and propagation
    pub constfold: bool,
    /// Enable devirtualization (replace virtual calls with direct calls)
    pub devirtualize: bool,
    /// Enable property flattening (inline trivial @property getters)
    pub flatten_properties: bool,
    /// Include debug information
    pub debug: bool,
    /// Verbose output
    pub verbose: bool,
    /// Emit HIR to stdout
    pub emit_hir: bool,
    /// Emit MIR to stdout
    pub emit_mir: bool,
    /// Emit the TypeInferencePass TypeTable (flow-sensitive SSA types) to stdout.
    /// Phase 1 §1.4 debug aid — lets callers inspect what the pass inferred
    /// before any consumer picks it up.
    pub emit_types: bool,
    /// Target triple (None = host)
    pub target: Option<String>,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            input: PathBuf::new(),
            output: PathBuf::new(),
            runtime_lib: PathBuf::from("target/release/libpyaot_runtime.a"),
            module_paths: Vec::new(),
            inline: false,
            inline_threshold: 50,
            dce: false,
            constfold: false,
            devirtualize: false,
            flatten_properties: false,
            debug: false,
            verbose: false,
            emit_hir: false,
            emit_mir: false,
            emit_types: false,
            target: None,
        }
    }
}

/// Debug-build SSA invariant gate. Walks every function in `module`
/// and runs `pyaot_mir::ssa_check::check`. On violation, `panic!`s
/// with the full violation list and the provided `where_` label so
/// the offending pipeline stage is identifiable. Compiled out of
/// release builds entirely.
#[cfg(debug_assertions)]
fn debug_assert_ssa(module: &pyaot_mir::Module, where_: &str) {
    for (func_id, func) in &module.functions {
        if let Err(violations) = pyaot_mir::ssa_check::check(func) {
            let formatted = violations
                .iter()
                .map(|v| format!("  - {}", v))
                .collect::<Vec<_>>()
                .join("\n");
            panic!(
                "SSA invariant violations ({}) in function {} ({}):\n{}",
                where_, func_id, func.name, formatted
            );
        }
    }
}

#[cfg(not(debug_assertions))]
#[inline(always)]
fn debug_assert_ssa(_module: &pyaot_mir::Module, _where_: &str) {}

/// Compile a Python source file to a native executable.
///
/// This runs the full pipeline: parse → semantic analysis → type check →
/// lower to MIR → optimize → codegen → link.
pub fn compile_to_executable(options: &CompileOptions) -> Result<()> {
    let target = if let Some(ref t) = options.target {
        t.parse::<Triple>()
            .map_err(|e| miette::miette!("Invalid target triple: {:?}", e))?
    } else {
        Triple::host()
    };

    if options.verbose {
        println!("Python AOT Compiler");
        println!("Input: {:?}", options.input);
        println!("Target: {}", target);
    }

    let module_name = options
        .input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("module")
        .to_string();

    // Set up search paths - include the directory containing the input file
    let mut search_paths = options.module_paths.clone();
    if let Some(parent) = options.input.parent() {
        if !parent.as_os_str().is_empty() {
            search_paths.insert(0, parent.to_path_buf());
        } else {
            search_paths.insert(0, PathBuf::from("."));
        }
    }

    // Append bundled `site-packages/` locations so Python packages shipped
    // with the compiler (e.g. `requests`) are importable without
    // `--module-path`. Candidates, in priority order:
    //   1. `<exe_dir>/site-packages`  — for installed / copied binaries
    //   2. `<repo_root>/site-packages` — dev fallback baked in at compile time
    // User-supplied paths and the input file's parent still win because they
    // were pushed before this block.
    let site_packages_candidates = [
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("site-packages"))),
        Some(PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../site-packages"
        ))),
    ];
    for root in site_packages_candidates.into_iter().flatten() {
        if root.is_dir() {
            search_paths.push(root);
        }
    }

    // Create module discovery
    let mut discovery = module_discovery::ModuleDiscovery::new(search_paths, options.verbose);

    // Discover all modules
    if options.verbose {
        println!("Discovering modules...");
    }
    discovery.discover_modules(&module_name, &options.input)?;

    // Topological sort
    let sorted_modules = discovery.topological_sort(&module_name);
    if options.verbose {
        println!("Module order: {:?}", sorted_modules);
    }

    // Check if we have multi-module compilation
    let has_imports = sorted_modules.len() > 1;

    // Get parsed modules
    let parsed_modules = discovery.take_modules();

    // Collect package imports across every parsed module before the HIR is
    // consumed by lowering. Each name maps onto a `libpyaot_pkg_<name>.a`
    // archive that will be passed to the linker below.
    let mut used_packages: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for module in parsed_modules.values() {
        for pkg in &module.hir.used_packages {
            used_packages.insert(pkg.clone());
        }
    }

    // Compile modules (single or multi)
    let (mut mir_module, mut interner) = if has_imports {
        if options.verbose {
            println!("Compiling {} modules...", sorted_modules.len());
        }
        mir_merger::MirMerger::compile_modules(
            parsed_modules,
            &sorted_modules,
            &module_name,
            options.verbose,
        )?
    } else {
        // Single module - process using pipeline
        let parsed = parsed_modules
            .into_iter()
            .next()
            .expect("single module must have at least one parsed module")
            .1;
        pipeline::compile_single_module(parsed, options.emit_hir, options.verbose)?
    };

    // Run optimizations
    let opt_config = pyaot_optimizer::OptimizeConfig {
        devirtualize: options.devirtualize,
        flatten_properties: options.flatten_properties,
        inline: options.inline,
        inline_threshold: options.inline_threshold,
        dce: options.dce,
        constfold: options.constfold,
    };
    if options.verbose {
        if opt_config.devirtualize {
            println!("Running devirtualization...");
        }
        if opt_config.flatten_properties {
            println!("Running property flattening...");
        }
        if opt_config.inline {
            println!(
                "Running function inlining optimization (threshold: {})...",
                opt_config.inline_threshold
            );
        }
        if opt_config.constfold {
            println!("Running constant folding and propagation...");
        }
        if opt_config.dce {
            println!("Running dead code elimination...");
        }
    }
    // S1.14b-prep pipeline order (2026-04-18): SSA construction runs
    // BEFORE optimize_module so every optimizer pass sees SSA MIR. A
    // debug-only SSA check gate fires after each structural mutation
    // so any future pass that breaks invariance surfaces at its own
    // site rather than silently.
    for func in mir_module.functions.values_mut() {
        pyaot_mir::ssa_construct::construct_ssa(func);
    }
    debug_assert_ssa(&mir_module, "post-construct_ssa");

    pyaot_optimizer::optimize_module(&mut mir_module, &opt_config, &mut interner);
    debug_assert_ssa(&mir_module, "post-optimize");

    if options.emit_mir {
        println!("MIR: {:#?}", mir_module);
    }

    // Flow-sensitive type inference over SSA MIR (Phase 1 §1.4).
    // Builds a TypeTable that WPA parameter inference then refines
    // using the call graph. No downstream codegen consumer reads this
    // yet — the pass runs purely for validation and for the optional
    // `--emit-types` debug dump. S1.9 wires lowering to query the
    // table in place of the legacy HIR-level type maps.
    if options.emit_types || options.verbose {
        let call_graph = pyaot_optimizer::call_graph::CallGraph::build(&mir_module);
        let mut type_table = pyaot_optimizer::type_inference::TypeTable::infer_module(&mir_module);
        // WPA params + fields together, to a whole-program fixed point
        // (§1.7 / S1.12). Field inference reads post-param-WPA types
        // from the TypeTable; param inference re-runs on every outer
        // iteration so that refined field types (when their consumers
        // land) can propagate back into caller arg-type observations.
        pyaot_optimizer::type_inference::wpa_param_and_field_inference_to_fixed_point(
            &mut mir_module,
            &call_graph,
            &mut type_table,
        );
        if options.emit_types {
            println!("TypeTable: {:#?}", type_table);
            println!("ClassInfo: {:#?}", mir_module.class_info);
        }
        if options.verbose {
            println!("Type inference + WPA (params + fields) complete.");
        }
    }

    // Codegen
    if options.verbose {
        println!("Generating code...");
    }
    let codegen = pyaot_codegen_cranelift::Codegen::new(target, options.debug).into_diagnostic()?;

    // Always create SourceInfo (needed for tracebacks; also used for DWARF debug info)
    let source_info = Some(pyaot_codegen_cranelift::SourceInfo {
        filename: options
            .input
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown.py")
            .to_string(),
        directory: options
            .input
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or(".")
            .to_string(),
        source: fs::read_to_string(&options.input).into_diagnostic()?,
    });

    let object_code = codegen
        .compile_module(&mir_module, &interner, source_info.as_ref())
        .into_diagnostic()?;

    // Write object file
    let obj_path = options.output.with_extension("o");
    fs::write(&obj_path, &object_code).into_diagnostic()?;

    if options.verbose {
        println!("Object file written to: {:?}", obj_path);
    }

    // Link
    if options.verbose {
        println!("Linking...");
    }
    let linker = pyaot_linker::Linker::with_debug(&options.runtime_lib, options.debug);

    // Resolve used-package names onto `.a` archive paths that sit alongside
    // the runtime library. This is the selective-linking step: a package's
    // archive is only added to the linker command when the source actually
    // imports it.
    let pkg_archives: Vec<PathBuf> = if used_packages.is_empty() {
        Vec::new()
    } else {
        let pkg_dir = options
            .runtime_lib
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        used_packages
            .iter()
            .map(|name| pkg_dir.join(format!("libpyaot_pkg_{}.a", name)))
            .collect()
    };
    if options.verbose && !pkg_archives.is_empty() {
        println!("Package archives: {:?}", pkg_archives);
    }

    linker
        .link(&obj_path, &options.output, &pkg_archives)
        .into_diagnostic()?;

    if options.verbose {
        println!("Executable written to: {:?}", options.output);
        println!("Compilation successful: {:?}", options.output);
    }

    Ok(())
}
