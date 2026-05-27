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

/// Per-stage MIR verifier mode. Stage A.1 of the Strong-Typed MIR Rewrite
/// (coordinated plan v2): each pipeline stage chooses independently
/// whether MIR violations panic, log to stderr, or pass silently.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerifyMode {
    /// Verifier not run at this stage.
    Off,
    /// Violations printed to stderr, compilation continues.
    Warn,
    /// Violations panic in debug builds; logged to stderr in release.
    HardError,
}

/// Per-stage verifier configuration. Stage A.1 of plan v2.
///
/// Default policy:
///   * In debug builds — every stage runs in `Warn` mode, except
///     `final_pre_codegen` which is `HardError`.
///   * In release builds — every stage runs in `Off` mode, except
///     `final_pre_codegen` which is `HardError` (Stage G.1: all 38
///     examples are verifier-clean, so hard-error is safe in release
///     too — violations surface immediately rather than silently).
///
/// Callers can override individual stages via [`CompileOptions::verify_mir_stages`]
/// or by setting the [`CompileOptions::verify_mir`] shortcut, which enables
/// `Warn` mode at every stage (preserving the prior `--verify-mir` behavior).
#[derive(Clone, Copy, Debug)]
pub struct VerifyMirConfig {
    pub post_lowering: VerifyMode,
    pub post_wpa_pass_1: VerifyMode,
    pub post_optimize: VerifyMode,
    pub post_mono: VerifyMode,
    pub final_pre_codegen: VerifyMode,
}

impl VerifyMirConfig {
    /// Default config — final-pre-codegen always-on, others off (warn in
    /// debug only at the boundary). Matches policy in module docs above.
    pub fn default_policy() -> Self {
        #[cfg(debug_assertions)]
        {
            Self {
                post_lowering: VerifyMode::Warn,
                post_wpa_pass_1: VerifyMode::Warn,
                post_optimize: VerifyMode::Warn,
                post_mono: VerifyMode::Warn,
                final_pre_codegen: VerifyMode::HardError,
            }
        }
        #[cfg(not(debug_assertions))]
        {
            Self {
                post_lowering: VerifyMode::Off,
                post_wpa_pass_1: VerifyMode::Off,
                post_optimize: VerifyMode::Off,
                post_mono: VerifyMode::Off,
                // Stage G.1: promote to HardError in release — all 38 examples
                // are verifier-clean at final-pre-codegen, so violations are
                // compiler bugs and should fail loudly rather than silently.
                final_pre_codegen: VerifyMode::HardError,
            }
        }
    }

    /// `--verify-mir` shortcut: enable Warn at every stage, keep
    /// HardError at `final_pre_codegen` (Stage G.1: unconditional in
    /// both debug and release).
    pub fn all_warn() -> Self {
        Self {
            post_lowering: VerifyMode::Warn,
            post_wpa_pass_1: VerifyMode::Warn,
            post_optimize: VerifyMode::Warn,
            post_mono: VerifyMode::Warn,
            final_pre_codegen: VerifyMode::HardError,
        }
    }

    pub fn for_stage(&self, stage: &str) -> VerifyMode {
        match stage {
            "post-lowering" => self.post_lowering,
            "post-wpa-pass-1" => self.post_wpa_pass_1,
            "post-optimize" => self.post_optimize,
            "post-mono" => self.post_mono,
            "final-pre-codegen" => self.final_pre_codegen,
            _ => VerifyMode::Off,
        }
    }
}

impl Default for VerifyMirConfig {
    fn default() -> Self {
        Self::default_policy()
    }
}

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
    /// Backwards-compatible shortcut: when true, enables `Warn` at every
    /// pipeline stage (matching the prior `--verify-mir` behavior). When
    /// false, the per-stage `verify_mir_stages` config is used.
    pub verify_mir: bool,
    /// Per-stage verifier configuration. Stage A.1 of Strong-Typed MIR
    /// Rewrite plan v2. Overrides `verify_mir` when set explicitly.
    pub verify_mir_stages: VerifyMirConfig,
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
            verify_mir: false,
            verify_mir_stages: VerifyMirConfig::default_policy(),
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

/// Strong-Typed MIR Verifier hook. Stage A.1 of the coordinated plan v2:
/// per-stage mode is consulted via [`VerifyMirConfig::for_stage`]. Three
/// modes are supported:
///
/// * [`VerifyMode::Off`] — no verifier run; instantly returns.
/// * [`VerifyMode::Warn`] — runs the verifier, prints violations to stderr,
///   compilation continues regardless of violation count.
/// * [`VerifyMode::HardError`] — runs the verifier; any violation panics with
///   a descriptive message. Stage G.1: HardError is now active in both debug
///   and release builds — all 38 examples are verifier-clean, so violations
///   are compiler bugs and must fail immediately.
///
/// `final-pre-codegen` always runs unconditionally regardless of the
/// `--verify-mir` flag (Phase 6e baseline). Stage A.1 keeps that behavior
/// by default — `VerifyMirConfig::default_policy()` sets `HardError` at
/// that stage in both debug and release builds (Stage G.1).
fn verify_mir_at(module: &pyaot_mir::Module, stage: &str, config: VerifyMirConfig) {
    let mode = config.for_stage(stage);
    if mode == VerifyMode::Off {
        return;
    }
    if let Err(errors) = pyaot_mir::verify_mir(module) {
        pyaot_mir::report_warnings(stage, &errors);
        if mode == VerifyMode::HardError && !errors.is_empty() {
            panic!(
                "[mir verifier hard-error @ {}]: {} violation(s) — \
                 MIR must be verifier-clean at this boundary. \
                 See stderr above for details.",
                stage,
                errors.len()
            );
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

    // Stage A.1: resolve effective per-stage verifier config. `--verify-mir`
    // (legacy boolean) overrides to all-warn; otherwise the explicit
    // `verify_mir_stages` is consulted.
    let verify_cfg = if options.verify_mir {
        VerifyMirConfig::all_warn()
    } else {
        options.verify_mir_stages
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
        raw_demotion: true,
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
    // Phase 2 (Strong-Typed MIR Rewrite): normalise Phi nodes by
    // inserting BoxValue at predecessor blocks for Raw sources merging
    // into Tagged dests. Closes the Phi-source-mismatch violations
    // flagged by the Verifier post-lowering.
    pyaot_mir::phi_normalize::normalize_phi_sources_module(&mut mir_module);
    debug_assert_ssa(&mir_module, "post-phi-normalize");
    verify_mir_at(&mir_module, "post-lowering", verify_cfg);

    if options.verbose {
        println!("Running mandatory SSA type analysis (pre-opt, pass 1)...");
    }
    // Phase 1 strict integration: run SSA type inference + WPA as a
    // mandatory production pass, repair call-site ABI from the
    // materialized types, then re-analyze the rewritten MIR so every
    // downstream consumer sees a single canonical view.
    pyaot_optimizer::type_inference::analyze_and_materialize_types(&mut mir_module);
    // Phase 2: re-normalise Phi sources after WPA widens dest types to
    // Tagged for heterogeneous merges. WPA may turn Heap(Str) dests into
    // Tagged (because sources couldn't agree on a narrower type), which
    // creates new Raw → Tagged mismatches that the initial post-lowering
    // run didn't see.
    pyaot_mir::phi_normalize::normalize_phi_sources_module(&mut mir_module);
    verify_mir_at(&mir_module, "post-wpa-pass-1", verify_cfg);
    // Pre-mono devirt: resolves CallVirtual on class-typed receivers to
    // CallDirect(template), so MonomorphizePass can specialize methods that
    // use Type::Var (e.g. unwrap(self) -> T → unwrap@<Int>).
    //
    // Safe because lower_class_method_call now packs *args before emitting
    // CallVirtual (matching the callee ABI), so post-devirt CallDirect has
    // the right arity for abi_repair to operate on.
    pyaot_optimizer::devirtualize::devirtualize(&mut mir_module);
    if options.verbose {
        println!("Running monomorphization pass...");
    }
    pyaot_optimizer::monomorphize::run(&mut mir_module, &mut interner);
    if options.verbose {
        println!("Repairing MIR ABI from materialized types (pre-opt)...");
    }
    pyaot_optimizer::abi_repair::repair_mir_abi_from_types(&mut mir_module).into_diagnostic()?;
    debug_assert_ssa(&mir_module, "post-abi-repair-pre-opt");
    // Phase 4 Commit 4 (Storage-Uniform): bridge tagged-Value returns
    // from flipped callees to their raw primitive dest locals. Run
    // after pre-opt `abi_repair` so cross-module `CallNamed` has been
    // narrowed to `CallDirect` (which the rewriter inspects), and
    // before the second `type_inference` pass so the retyped temp/dest
    // pair is in place for WPA's seed pass.
    //
    // Stage E.3 follow-up (2026-05): @property getter scenario (was case 1
    // in the original audit) is now fixed: is_class_method in
    // function_lowering.rs and phase4_safe_scan.rs now checks
    // ClassDef::properties in addition to ClassDef::methods, so property
    // getters are correctly classified as class methods and excluded from
    // phase4_return_abi_flipped. Two load-bearing scenarios remain:
    //   1. Generator resume functions: unconditionally marked
    //      phase4_return_abi_flipped = true; callers that unwrap the
    //      iterator Value need the UnboxValue bridge.
    //   2. Lambdas / nested functions with annotated primitive returns that
    //      are phase4_safe (not used as HOF callbacks): they flip their
    //      return ABI so callers' dest locals need the bridge.
    // Removing this call causes 12 test failures (verified 2026-05).
    // The companion WPA guard in type_inference::materialize_function_return_types
    // (skips when phase4_return_abi_flipped) is equally load-bearing —
    // removing it causes WPA to re-narrow Type::Any → Int/Float for
    // flipped functions.
    pyaot_lowering::rewrite_phase4_callee_returns(&mut mir_module);
    if options.verbose {
        println!("Running mandatory SSA type analysis (pre-opt, pass 2)...");
    }
    pyaot_optimizer::type_inference::analyze_and_materialize_types(&mut mir_module);
    // Stage B.4 of Strong-Typed MIR Rewrite plan v2: defensive sweep
    // `rebox_tagged_any_copies` is now subsumed by Phase 3a-* monomorph
    // mir_ty syncs + box_fusion. The helper and this call have been
    // deleted; the comment is preserved as an anchor for the rationale.
    // Invariant: after the second WPA pass, caller locals produced by
    // generic calls are fully resolved — no Type::Var should remain.
    #[cfg(debug_assertions)]
    pyaot_optimizer::monomorphize::assert_no_var_remaining(&mir_module);

    pyaot_optimizer::optimize_module(&mut mir_module, &opt_config, &mut interner);
    debug_assert_ssa(&mir_module, "post-optimize");
    verify_mir_at(&mir_module, "post-optimize", verify_cfg);

    // Re-run after optimization so codegen sees final local/param/field
    // types after inlining / const-folding / devirtualization rewrites,
    // then repair the final call ABI once more for the post-opt MIR.
    if options.verbose {
        println!("Running mandatory SSA type analysis (post-opt, pass 1)...");
    }
    pyaot_optimizer::type_inference::analyze_and_materialize_types(&mut mir_module);
    if options.verbose {
        println!("Repairing MIR ABI from materialized types (post-opt)...");
    }
    pyaot_optimizer::abi_repair::repair_mir_abi_from_types(&mut mir_module).into_diagnostic()?;
    debug_assert_ssa(&mir_module, "post-abi-repair-final");
    if options.verbose {
        println!("Running mandatory SSA type analysis (post-opt, pass 2)...");
    }
    let final_type_table =
        pyaot_optimizer::type_inference::analyze_and_materialize_types(&mut mir_module);
    // Final phi_normalize: WPA may have narrowed Phi-merged locals'
    // declared types post-opt; re-normalize to catch any new Raw→Tagged
    // mismatches and narrow Phi dests with uniform Raw sources.
    pyaot_mir::phi_normalize::normalize_phi_sources_module(&mut mir_module);
    // Cleanup: when WPA's `refine_function_params` narrowed a param's
    // `mir_ty` from `Tagged` to `Raw(K)` (because call-site analysis
    // found a concrete primitive), any `UnboxValue` that lowering had
    // emitted on that param at a time when it was still `Tagged` now
    // has a Raw-shaped source, which violates the verifier's Tagged
    // contract. Rewrite such UnboxValues to `Copy`.
    pyaot_optimizer::peephole::run_redundant_unbox_cleanup(&mut mir_module);
    // Stage G.1: final-pre-codegen verifier ALWAYS runs in HardError mode
    // (forced on even when config sets Off for safety — 38 examples are
    // verifier-clean, so violations are compiler bugs and must fail
    // immediately in both debug and release builds).
    let final_cfg = if verify_cfg.final_pre_codegen == VerifyMode::Off {
        let mut c = verify_cfg;
        c.final_pre_codegen = VerifyMode::HardError;
        c
    } else {
        verify_cfg
    };
    verify_mir_at(&mir_module, "final-pre-codegen", final_cfg);

    if options.emit_mir {
        println!("MIR: {:#?}", mir_module);
    }

    if options.emit_types {
        println!("TypeTable: {:#?}", final_type_table);
        println!("ClassInfo: {:#?}", mir_module.class_info);
    }
    if options.verbose {
        println!("Type inference + WPA (params + fields) materialized.");
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
