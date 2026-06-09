//! # pyaot — compiler entry point
//!
//! Orchestrates the pipeline:
//!
//! ```text
//! source ─▶ frontend-python ─▶ HIR ─▶ semantics ─▶ typeck ─▶ lowering(+legalize)
//!        ─▶ MIR(verify) ─▶ optimizer(verify) ─▶ codegen-cranelift ─▶ linker ─▶ exe
//! ```

#![forbid(unsafe_code)]

use std::path::PathBuf;

use clap::Parser;

use pyaot_diagnostics::{CompilerError, Result};
use pyaot_utils::StringInterner;

/// Static AOT compiler for a typed subset of Python 3 → native (Cranelift).
#[derive(Parser)]
#[command(name = "pyaot", version, about)]
struct Cli {
    /// Input `.py` source file.
    input: PathBuf,
    /// Output executable path.
    #[arg(short = 'o', long = "output")]
    output: PathBuf,
    /// Path to `libpyaot_runtime.a` (overrides auto-detection).
    #[arg(long = "runtime-lib")]
    runtime_lib: Option<PathBuf>,
    /// Keep debug symbols / DWARF (no stripping).
    #[arg(long)]
    debug: bool,
}

fn main() {
    let cli = Cli::parse();

    let source = match std::fs::read_to_string(&cli.input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("pyaot: cannot read {}: {e}", cli.input.display());
            std::process::exit(1);
        }
    };

    if let Err(err) = compile(&cli, &source) {
        eprint!("{}", err.format(&cli.input.display().to_string(), &source));
        std::process::exit(1);
    }
}

fn compile(cli: &Cli, source: &str) -> Result<()> {
    let mut interner = StringInterner::new();

    // ── front-half ──
    let mut module = pyaot_frontend_python::parse(source, &mut interner)?;
    let resolve = pyaot_semantics::resolve(&mut module, &interner)?;
    pyaot_typeck::infer(&mut module, &resolve)?;
    let mut mir = pyaot_lowering::lower(&module, &resolve, &interner)?;

    // ── verify after lowering (debug): the first MIR is checked before any pass. ──
    #[cfg(debug_assertions)]
    {
        for func in &mir.funcs {
            pyaot_mir::verify(func, &mir.funcs).map_err(verify_to_error)?;
        }
    }

    // ── optimizer (empty Phase 1 pipeline; verifies at the boundary). ──
    let passes = pyaot_optimizer::PassManager::phase1();
    passes.run(&mut mir).map_err(verify_to_error)?;

    // ── codegen → object → link. ──
    let object_path = cli.output.with_extension("o");
    pyaot_codegen_cranelift::compile(&mir, &object_path)?;

    let runtime_lib = locate_runtime_lib(cli)?;
    let linker = pyaot_linker::Linker::with_debug(runtime_lib, cli.debug);
    linker.link(&object_path, &cli.output, &[])?;

    Ok(())
}

fn verify_to_error(e: pyaot_mir::VerifyError) -> CompilerError {
    CompilerError::codegen_error(format!("MIR verification failed: {e}"), None)
}

/// Locate `libpyaot_runtime.a` by precedence: `--runtime-lib` → `PYAOT_RUNTIME_LIB`
/// → the compiler's own `target/<profile>/` (so it matches the build profile).
fn locate_runtime_lib(cli: &Cli) -> Result<PathBuf> {
    const LIB_NAME: &str = "libpyaot_runtime.a";

    if let Some(path) = &cli.runtime_lib {
        return if path.exists() {
            Ok(path.clone())
        } else {
            Err(CompilerError::link_error(format!(
                "runtime library not found at --runtime-lib path: {}",
                path.display()
            )))
        };
    }

    if let Ok(env_path) = std::env::var("PYAOT_RUNTIME_LIB") {
        let path = PathBuf::from(env_path);
        return if path.exists() {
            Ok(path)
        } else {
            Err(CompilerError::link_error(format!(
                "PYAOT_RUNTIME_LIB points to a missing file: {}",
                path.display()
            )))
        };
    }

    // Derive from the compiler's own location: `target/<profile>/pyaot` lives
    // next to `target/<profile>/libpyaot_runtime.a`, matching our build profile.
    let exe = std::env::current_exe().map_err(|e| {
        CompilerError::link_error(format!("cannot locate the pyaot executable: {e}"))
    })?;
    let candidate = exe
        .parent()
        .map(|dir| dir.join(LIB_NAME))
        .ok_or_else(|| CompilerError::link_error("pyaot executable has no parent directory"))?;

    if candidate.exists() {
        Ok(candidate)
    } else {
        Err(CompilerError::link_error(format!(
            "could not find {LIB_NAME} (looked at {}). Build it with \
             `cargo build -p pyaot-runtime` (PITFALLS B9), or pass --runtime-lib / \
             set PYAOT_RUNTIME_LIB.",
            candidate.display()
        )))
    }
}
