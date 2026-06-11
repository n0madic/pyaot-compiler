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

use clap::ValueEnum;

use pyaot_codegen_cranelift::{CodegenOptions, OptLevel};
use pyaot_diagnostics::{CompilerError, Result};
use pyaot_frontend_python::ModuleSource;
use pyaot_utils::StringInterner;

/// Resolves `import` targets against the entry script's directory (Phase 8): a
/// dotted module `a.b.c` is `root/a/b/c.py`, else the package `root/a/b/c/__init__.py`.
struct DirModuleSource {
    root: PathBuf,
}

impl ModuleSource for DirModuleSource {
    fn load(&mut self, path: &[String]) -> Option<(String, bool)> {
        let mut base = self.root.clone();
        for component in path {
            base.push(component);
        }
        let module_file = base.with_extension("py");
        if module_file.is_file() {
            return std::fs::read_to_string(&module_file).ok().map(|s| (s, false));
        }
        let init_file = base.join("__init__.py");
        if init_file.is_file() {
            return std::fs::read_to_string(&init_file).ok().map(|s| (s, true));
        }
        None
    }
}

/// Static AOT compiler for a typed subset of Python 3 → native (Cranelift).
#[derive(Parser)]
#[command(name = "pyaot", version, about)]
struct Cli {
    /// Input `.py` source file.
    input: PathBuf,
    /// Output executable path. Optional when `--emit-mir` is given.
    #[arg(short = 'o', long = "output")]
    output: Option<PathBuf>,
    /// Path to `libpyaot_runtime.a` (overrides auto-detection).
    #[arg(long = "runtime-lib")]
    runtime_lib: Option<PathBuf>,
    /// Keep debug symbols / DWARF (no stripping). Also defaults the
    /// optimization level to `none` (predictable stepping) unless an explicit
    /// `--opt-level` overrides it.
    #[arg(long)]
    debug: bool,
    /// Optimization level: `none` (fully conservative — empty MIR pipeline +
    /// Cranelift opt_level=none), `speed` (default), or `speed-and-size`.
    #[arg(long = "opt-level", value_enum)]
    opt_level: Option<OptLevelArg>,
    /// Escape hatch for PITFALLS B17: disable Cranelift alias analysis under
    /// `--opt-level speed` (see CodegenOptions in pyaot-codegen-cranelift).
    #[arg(long = "no-alias-analysis", hide = true)]
    no_alias_analysis: bool,
    /// Print the lowered, verified MIR to stdout and exit (a debug aid for
    /// confirming representation specialization — e.g. unboxed `Raw(F64)`
    /// arithmetic — that the differential gate cannot distinguish by output).
    #[arg(long = "emit-mir")]
    emit_mir: bool,
}

/// `--opt-level` values (clap-facing mirror of [`OptLevel`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum OptLevelArg {
    None,
    Speed,
    SpeedAndSize,
}

impl Cli {
    /// Resolve the effective optimization level: explicit `--opt-level` wins;
    /// otherwise `--debug` means `none` and the default is `speed`.
    fn effective_opt_level(&self) -> OptLevelArg {
        match self.opt_level {
            Some(level) => level,
            None if self.debug => OptLevelArg::None,
            None => OptLevelArg::Speed,
        }
    }
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
    // The import search path is the entry script's directory (Phase 8).
    let root = cli
        .input
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let mut loader = DirModuleSource { root };
    let program = pyaot_frontend_python::parse_program(source, &mut loader, &mut interner)?;
    let mut module = program.module;
    let namespaces = program.namespaces;
    let resolve = pyaot_semantics::resolve(&mut module, &namespaces, &interner)?;
    let mut classes = pyaot_semantics::collect_classes(&module, &namespaces, &interner)?;
    pyaot_typeck::infer(&mut module, &resolve, &mut classes, &interner)?;
    let mut mir = pyaot_lowering::lower(&module, &resolve, &interner, &classes)?;

    // ── verify after lowering (debug): the first MIR is checked before any pass. ──
    #[cfg(debug_assertions)]
    {
        for func in &mir.funcs {
            pyaot_mir::verify(func, &mir.funcs).map_err(verify_to_error)?;
        }
    }

    // ── optimizer: `--opt-level none` is the single conservative switch (empty
    // MIR pipeline + Cranelift opt_level=none). ──
    let opt_level = cli.effective_opt_level();
    let passes = match opt_level {
        OptLevelArg::None => pyaot_optimizer::PassManager::phase1(),
        OptLevelArg::Speed | OptLevelArg::SpeedAndSize => pyaot_optimizer::PassManager::phase1(),
    };
    passes.run(&mut mir).map_err(verify_to_error)?;

    // ── --emit-mir: dump the verified MIR and stop (no codegen/link). ──
    if cli.emit_mir {
        for (i, func) in mir.funcs.iter().enumerate() {
            println!("// ── fn {i} ──");
            println!("{func:#?}");
        }
        return Ok(());
    }

    // ── codegen → object → link. ──
    let output = cli.output.as_ref().ok_or_else(|| {
        CompilerError::codegen_error("an output path (-o) is required unless --emit-mir is set", None)
    })?;
    let object_path = output.with_extension("o");
    let codegen_opts = CodegenOptions {
        opt_level: match opt_level {
            OptLevelArg::None => OptLevel::None,
            OptLevelArg::Speed => OptLevel::Speed,
            OptLevelArg::SpeedAndSize => OptLevel::SpeedAndSize,
        },
        alias_analysis: !cli.no_alias_analysis,
    };
    pyaot_codegen_cranelift::compile(&mir, &object_path, &codegen_opts)?;

    let runtime_lib = locate_runtime_lib(cli)?;
    let linker = pyaot_linker::Linker::with_debug(runtime_lib, cli.debug);
    linker.link(&object_path, output, &[])?;

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
