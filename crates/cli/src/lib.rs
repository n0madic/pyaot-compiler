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
use std::path::PathBuf;
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
    /// Include debug information
    pub debug: bool,
    /// Verbose output
    pub verbose: bool,
    /// Emit HIR to stdout
    pub emit_hir: bool,
    /// Emit MIR to stdout
    pub emit_mir: bool,
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
            debug: false,
            verbose: false,
            emit_hir: false,
            emit_mir: false,
            target: None,
        }
    }
}

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

    // Compile modules (single or multi)
    let (mut mir_module, interner) = if has_imports {
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
        inline: options.inline,
        inline_threshold: options.inline_threshold,
        dce: options.dce,
    };
    if options.verbose {
        if opt_config.inline {
            println!(
                "Running function inlining optimization (threshold: {})...",
                opt_config.inline_threshold
            );
        }
        if opt_config.dce {
            println!("Running dead code elimination...");
        }
    }
    pyaot_optimizer::optimize_module(&mut mir_module, &opt_config);

    if options.emit_mir {
        println!("MIR: {:#?}", mir_module);
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
    linker.link(&obj_path, &options.output).into_diagnostic()?;

    if options.verbose {
        println!("Executable written to: {:?}", options.output);
        println!("Compilation successful: {:?}", options.output);
    }

    Ok(())
}
