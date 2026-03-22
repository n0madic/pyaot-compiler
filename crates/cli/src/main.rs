//! Python AOT Compiler CLI

#![forbid(unsafe_code)]

mod args;

use clap::Parser;
use miette::{IntoDiagnostic, Result};
use pyaot::{compile_to_executable, CompileOptions};
use std::path::PathBuf;
use std::process::Command;

fn main() -> Result<()> {
    let args = args::Args::parse();

    // Determine output file
    let output = pyaot::pipeline::determine_output_path(&args.input, args.output);

    let options = CompileOptions {
        input: args.input,
        output: output.clone(),
        runtime_lib: args.runtime_lib,
        module_paths: args.module_path,
        inline: args.inline,
        inline_threshold: args.inline_threshold,
        debug: args.debug,
        verbose: args.verbose,
        emit_hir: args.emit_hir,
        emit_mir: args.emit_mir,
        target: args.target,
    };

    compile_to_executable(&options)?;

    // Run the executable if --run flag is set
    if args.run {
        // Ensure the path is executable - relative paths need "./" prefix on Unix
        let executable_path =
            if output.is_relative() && output.parent() == Some(std::path::Path::new("")) {
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

        std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
}
