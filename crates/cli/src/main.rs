//! Python AOT Compiler CLI

#![forbid(unsafe_code)]

mod args;
mod import_resolver;
mod mir_merger;
mod module_discovery;
mod pipeline;
mod types;

use clap::Parser;
use miette::{IntoDiagnostic, Result};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use target_lexicon::Triple;

fn main() -> Result<()> {
    let args = args::Args::parse();

    if args.verbose {
        println!("Python AOT Compiler");
        println!("Input: {:?}", args.input);
    }

    // Determine output file
    let output = pipeline::determine_output_path(&args.input, args.output);

    // Determine target
    let target = if let Some(t) = args.target {
        t.parse::<Triple>()
            .map_err(|e| miette::miette!("Invalid target triple: {:?}", e))?
    } else {
        Triple::host()
    };

    if args.verbose {
        println!("Target: {}", target);
    }

    let module_name = args
        .input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("module")
        .to_string();

    // Set up search paths - include the directory containing the input file
    let mut search_paths = args.module_path.clone();
    if let Some(parent) = args.input.parent() {
        if !parent.as_os_str().is_empty() {
            search_paths.insert(0, parent.to_path_buf());
        } else {
            search_paths.insert(0, PathBuf::from("."));
        }
    }

    // Create module discovery
    let mut discovery = module_discovery::ModuleDiscovery::new(search_paths, args.verbose);

    // Discover all modules
    if args.verbose {
        println!("Discovering modules...");
    }
    discovery.discover_modules(&module_name, &args.input)?;

    // Topological sort
    let sorted_modules = discovery.topological_sort(&module_name);
    if args.verbose {
        println!("Module order: {:?}", sorted_modules);
    }

    // Check if we have multi-module compilation
    let has_imports = sorted_modules.len() > 1;

    // Get parsed modules
    let parsed_modules = discovery.take_modules();

    // Compile modules (single or multi)
    let (mut mir_module, interner) = if has_imports {
        if args.verbose {
            println!("Compiling {} modules...", sorted_modules.len());
        }
        mir_merger::MirMerger::compile_modules(
            parsed_modules,
            &sorted_modules,
            &module_name,
            args.verbose,
        )?
    } else {
        // Single module - process using pipeline
        let parsed = parsed_modules
            .into_iter()
            .next()
            .expect("single module must have at least one parsed module")
            .1;
        pipeline::compile_single_module(parsed, args.emit_hir, args.verbose)?
    };

    // Run optimizations
    if args.inline {
        if args.verbose {
            println!(
                "Running function inlining optimization (threshold: {})...",
                args.inline_threshold
            );
        }
        pyaot_optimizer::inline::inline_functions(&mut mir_module, args.inline_threshold);
    }

    if args.emit_mir {
        println!("MIR: {:#?}", mir_module);
    }

    // Codegen
    if args.verbose {
        println!("Generating code...");
    }
    let codegen = pyaot_codegen_cranelift::Codegen::new(target, args.debug).into_diagnostic()?;

    let object_code = codegen
        .compile_module(&mir_module, &interner)
        .into_diagnostic()?;

    // Write object file
    let obj_path = output.with_extension("o");
    fs::write(&obj_path, &object_code).into_diagnostic()?;

    if args.verbose {
        println!("Object file written to: {:?}", obj_path);
    }

    // Link
    if args.verbose {
        println!("Linking...");
    }
    let linker = pyaot_linker::Linker::with_debug(&args.runtime_lib, args.debug);
    linker.link(&obj_path, &output).into_diagnostic()?;

    if args.verbose {
        println!("Executable written to: {:?}", output);
        println!("Compilation successful: {:?}", output);
    }

    // Run the executable if --run flag is set
    if args.run {
        // Ensure the path is executable - relative paths need "./" prefix on Unix
        let executable_path =
            if output.is_relative() && output.parent() == Some(std::path::Path::new("")) {
                // Path like "test_out" needs to become "./test_out"
                PathBuf::from(".").join(&output)
            } else {
                output.clone()
            };

        if args.verbose {
            println!("\nRunning: {:?}", executable_path);
            println!("----------------------------------------");
        }

        let status = Command::new(&executable_path)
            .status()
            .into_diagnostic()
            .map_err(|e| {
                eprintln!("Failed to run executable: {}", e);
                e
            })?;

        if args.verbose {
            println!("----------------------------------------");
            println!("Exit code: {}", status.code().unwrap_or(-1));
        }

        // Exit with the same code as the compiled program
        std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
}
