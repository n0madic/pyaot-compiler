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
use pyaot_mir::MirProgram;
use pyaot_utils::StringInterner;

/// Resolves `import` targets against a list of search roots (Phase 8): a dotted
/// module `a.b.c` is `root/a/b/c.py`, else the package `root/a/b/c/__init__.py`.
/// Roots are tried in order — `roots[0]` is the entry script's directory,
/// followed by any `--module-path` directories — and the first match wins.
struct DirModuleSource {
    roots: Vec<PathBuf>,
}

impl ModuleSource for DirModuleSource {
    fn load(&mut self, path: &[String]) -> Option<(String, bool)> {
        for root in &self.roots {
            let mut base = root.clone();
            for component in path {
                base.push(component);
            }
            let module_file = base.with_extension("py");
            if module_file.is_file() {
                return std::fs::read_to_string(&module_file)
                    .ok()
                    .map(|s| (s, false));
            }
            let init_file = base.join("__init__.py");
            if init_file.is_file() {
                return std::fs::read_to_string(&init_file).ok().map(|s| (s, true));
            }
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
    /// Output executable path. Defaults to the input path with its extension
    /// stripped (`foo.py` → `foo`).
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
    /// Additional directories to search for imported modules (repeatable).
    #[arg(long = "module-path", value_name = "DIR")]
    module_path: Vec<PathBuf>,
    /// Enable all optimizations (alias for `--opt-level speed`).
    #[arg(short = 'O', long = "optimize")]
    optimize: bool,
    /// Print the resolved HIR to stdout and exit (no typeck/codegen).
    #[arg(long = "emit-hir")]
    emit_hir: bool,
    /// Print the HIR with inferred types to stdout and exit (no codegen).
    #[arg(long = "emit-types")]
    emit_types: bool,
    /// Verbose progress output to stderr.
    #[arg(short = 'v', long = "verbose")]
    verbose: bool,
    /// Run the compiled executable immediately after a successful link.
    #[arg(long = "run")]
    run: bool,
}

/// `--opt-level` values (clap-facing mirror of [`OptLevel`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum OptLevelArg {
    None,
    Speed,
    SpeedAndSize,
}

impl Cli {
    /// Resolve the effective optimization level. Precedence: explicit
    /// `--opt-level` wins; then `-O/--optimize` forces `speed`; then `--debug`
    /// means `none`; otherwise the default is `speed`. `-O` only gates the
    /// `phase1`/`phase9` choice — the canonical pass order in `phase9` is never
    /// touched.
    fn effective_opt_level(&self) -> OptLevelArg {
        match self.opt_level {
            Some(level) => level,
            None if self.optimize => OptLevelArg::Speed,
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
    // `-v/--verbose`: announce each pipeline stage on stderr. A no-op closure
    // keeps the discrete steps below unchanged when verbose is off.
    let step = |msg: &str| {
        if cli.verbose {
            eprintln!("pyaot: {msg}");
        }
    };

    let mut interner = StringInterner::new();

    // ── front-half ──
    // The import search path starts with the entry script's directory (Phase 8),
    // then any `--module-path` directories, in order.
    let mut roots = vec![cli
        .input
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))];
    roots.extend(cli.module_path.iter().cloned());
    let mut loader = DirModuleSource { roots };
    step("Parsing");
    let program = pyaot_frontend_python::parse_program(
        source,
        &cli.input.display().to_string(),
        &mut loader,
        &mut interner,
    )?;
    let mut module = program.module;
    let namespaces = program.namespaces;
    step("Resolving");
    let resolve = pyaot_semantics::resolve(&mut module, &namespaces, &interner)?;

    // ── --emit-hir: dump the resolved HIR and stop (types are still `Dyn`). ──
    if cli.emit_hir {
        println!("{module:#?}");
        return Ok(());
    }

    step("Collecting classes");
    let mut classes = pyaot_semantics::collect_classes(&module, &namespaces, &interner)?;
    step("Type inference");
    pyaot_typeck::infer(&mut module, &resolve, &mut classes, &interner)?;

    // ── --emit-types: dump the HIR after inference (the `ty` fields are now
    // refined — `infer` mutates the HIR in place) and stop. ──
    if cli.emit_types {
        println!("{module:#?}");
        println!("{classes:#?}");
        return Ok(());
    }

    step("Lowering");
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
    step(&format!("Optimizing (opt-level {opt_level:?})"));
    let passes = match opt_level {
        OptLevelArg::None => pyaot_optimizer::PassManager::phase1(),
        OptLevelArg::Speed | OptLevelArg::SpeedAndSize => pyaot_optimizer::PassManager::phase9(),
    };
    passes.run(&mut mir).map_err(verify_to_error)?;

    // ── mandatory pre-codegen verify (release-safe, PLAN #2). ──
    verify_pre_codegen(&mir)?;

    // ── --emit-mir: dump the verified MIR and stop (no codegen/link). ──
    if cli.emit_mir {
        for (i, func) in mir.funcs.iter().enumerate() {
            println!("// ── fn {i} ──");
            println!("{func:#?}");
        }
        return Ok(());
    }

    // ── codegen → object → link. ──
    // Default the output to the input path with its extension stripped
    // (`foo.py` → `foo`) when `-o` is omitted.
    let output = cli.output.clone().unwrap_or_else(|| {
        let mut path = cli.input.clone();
        path.set_extension("");
        path
    });
    let object_path = output.with_extension("o");
    let codegen_opts = CodegenOptions {
        opt_level: match opt_level {
            OptLevelArg::None => OptLevel::None,
            OptLevelArg::Speed => OptLevel::Speed,
            OptLevelArg::SpeedAndSize => OptLevel::SpeedAndSize,
        },
        alias_analysis: !cli.no_alias_analysis,
    };
    step("Codegen");
    pyaot_codegen_cranelift::compile(&mir, &object_path, &codegen_opts, &interner)?;

    step("Linking");
    let runtime_lib = locate_runtime_lib(cli)?;
    let linker = pyaot_linker::Linker::with_debug(runtime_lib, cli.debug);
    linker.link(&object_path, &output, &[])?;
    step("Done");

    // ── --run: execute the freshly linked binary, propagating its exit code. ──
    if cli.run {
        if cli.verbose {
            eprintln!("pyaot: Running: {}", output.display());
        }
        let status = std::process::Command::new(&output)
            .status()
            .map_err(|e| {
                CompilerError::link_error(format!(
                    "failed to run the compiled executable {}: {e}",
                    output.display()
                ))
            })?;
        std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
}

/// The mandatory, release-safe pre-codegen representation gate (PLAN #2).
///
/// Runs in ALL build profiles — never wrap this call, or this function, in
/// `#[cfg(debug_assertions)]`. The per-pass-boundary verifier in `optimizer`
/// and the post-lowering verify above are debug-only; this one is NOT, so a
/// representation mismatch surfaces here as a hard `CompilerError` right before
/// codegen rather than a silent miscompile or SEGV — e.g. a wrong Phase-3c
/// interval that produced a raw divide of a value that is actually a bignum.
/// Linear in MIR — negligible cost.
fn verify_pre_codegen(mir: &MirProgram) -> Result<()> {
    for func in &mir.funcs {
        pyaot_mir::verify(func, &mir.funcs).map_err(verify_to_error)?;
    }
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

#[cfg(test)]
mod tests {
    use super::verify_pre_codegen;
    use pyaot_diagnostics::CompilerError;
    use pyaot_mir::{
        Const, LocalDecl, MirBlock, MirFunction, MirInst, MirProgram, MirTerminator, StrPool,
    };
    use pyaot_types::{HeapShape, Repr};
    use pyaot_utils::{BlockId, FuncId, LocalId, StringInterner};

    /// One-block function: `locals` declares the Repr table, `insts` the body,
    /// `term` the terminator. Mirrors `verify.rs`'s `single_block` test helper.
    fn single_block(locals: Vec<Repr>, insts: Vec<MirInst>, term: MirTerminator) -> MirFunction {
        let mut interner = StringInterner::new();
        MirFunction {
            name: interner.intern("__main__"),
            file: interner.intern("test.py"),
            params: Vec::new(),
            ret: Repr::Tagged,
            locals: locals.into_iter().map(|repr| LocalDecl { repr }).collect(),
            blocks: vec![MirBlock {
                insts,
                term,
                handler: None,
            }],
            entry: BlockId::new(0),
        }
    }

    /// Wrap one function as a whole program — the shape the CLI gate consumes.
    fn program(f: MirFunction) -> MirProgram {
        MirProgram {
            funcs: vec![f],
            entry: FuncId::new(0),
            str_pool: StrPool::new(),
            classes: Vec::new(),
            generators: Vec::new(),
        }
    }

    /// A program that materializes a string constant into `locals[0]`. `slot` is
    /// that local's Repr: `Heap(Str)` is well-formed; anything else is a
    /// representation mismatch the pre-codegen gate must reject.
    fn const_str_into(slot: Repr) -> MirProgram {
        let f = single_block(
            vec![slot],
            vec![MirInst::Const {
                dst: LocalId::new(0),
                val: Const::Str(StringInterner::new().intern("x")),
            }],
            MirTerminator::Return(None),
        );
        program(f)
    }

    #[test]
    fn pre_codegen_gate_accepts_well_formed() {
        // `Const::Str` into a `Heap(Str)` slot is the canonical good shape.
        assert!(verify_pre_codegen(&const_str_into(Repr::Heap(HeapShape::Str))).is_ok());
    }

    #[test]
    fn pre_codegen_gate_rejects_broken_repr() {
        // `Const::Str` into a `Tagged` slot is a representation mismatch. This
        // test is NOT `#[cfg]`-gated, so it also runs — and must pass — under
        // `cargo test --release`, proving the gate fires in release builds and
        // is wired to a hard `CompilerError`, not a debug-only assertion.
        let err = verify_pre_codegen(&const_str_into(Repr::Tagged))
            .expect_err("the pre-codegen gate must reject representation-broken MIR");
        match err {
            CompilerError::CodegenError { message, .. } => assert!(
                message.contains("MIR verification failed"),
                "unexpected error message: {message}"
            ),
            other => panic!("expected CodegenError, got {other:?}"),
        }
    }
}
