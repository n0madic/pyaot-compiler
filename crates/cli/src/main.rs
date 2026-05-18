//! Python AOT Compiler CLI

#![forbid(unsafe_code)]

mod args;

use clap::Parser;
use miette::{IntoDiagnostic, Result};
use pyaot::{compile_to_executable, CompileOptions, VerifyMirConfig, VerifyMode};
use std::path::PathBuf;
use std::process::Command;

fn parse_verify_mode(s: &str) -> Result<VerifyMode> {
    match s.to_ascii_lowercase().as_str() {
        "off" => Ok(VerifyMode::Off),
        "warn" | "warning" => Ok(VerifyMode::Warn),
        "hard-error" | "harderror" | "error" => Ok(VerifyMode::HardError),
        other => Err(miette::miette!(
            "invalid verifier mode '{}' (expected off|warn|hard-error)",
            other
        )),
    }
}

fn apply_stage_overrides(base: VerifyMirConfig, overrides: &[String]) -> Result<VerifyMirConfig> {
    let mut cfg = base;
    for spec in overrides {
        let (stage, mode_str) = spec.split_once('=').ok_or_else(|| {
            miette::miette!(
                "invalid --verify-mir-stage '{}' (expected STAGE=MODE)",
                spec
            )
        })?;
        let mode = parse_verify_mode(mode_str)?;
        match stage {
            "post-lowering" => cfg.post_lowering = mode,
            "post-wpa-pass-1" => cfg.post_wpa_pass_1 = mode,
            "post-optimize" => cfg.post_optimize = mode,
            "post-mono" => cfg.post_mono = mode,
            "final-pre-codegen" => cfg.final_pre_codegen = mode,
            other => {
                return Err(miette::miette!(
                    "unknown verifier stage '{}' (expected post-lowering|post-wpa-pass-1|post-optimize|post-mono|final-pre-codegen)",
                    other
                ))
            }
        }
    }
    Ok(cfg)
}

fn main() -> Result<()> {
    let args = args::Args::parse();

    // Determine output file
    let output = pyaot::pipeline::determine_output_path(&args.input, args.output);

    let base_verify_cfg = if args.verify_mir {
        VerifyMirConfig::all_warn()
    } else {
        VerifyMirConfig::default_policy()
    };
    let verify_mir_stages = apply_stage_overrides(base_verify_cfg, &args.verify_mir_stage)?;

    let options = CompileOptions {
        input: args.input,
        output: output.clone(),
        runtime_lib: args.runtime_lib,
        module_paths: args.module_path,
        inline: args.inline || args.optimize,
        inline_threshold: args.inline_threshold,
        dce: args.dce || args.optimize,
        constfold: args.constfold || args.optimize,
        devirtualize: args.devirtualize || args.optimize,
        flatten_properties: args.flatten_properties || args.optimize,
        debug: args.debug,
        verbose: args.verbose,
        emit_hir: args.emit_hir,
        emit_mir: args.emit_mir,
        emit_types: args.emit_types,
        verify_mir: args.verify_mir,
        verify_mir_stages,
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
