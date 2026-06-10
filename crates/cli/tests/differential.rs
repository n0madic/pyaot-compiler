//! Differential harness — the Phase-1 gate.
//!
//! For each file in [`PHASE1_CORPUS`] (an explicit **allowlist**, NOT a glob, so
//! the full-feature corpus files cannot break this phase's gate): compile it with
//! `pyaot`, run the resulting executable, run the same file under `python3`, and
//! assert the two stdouts are byte-for-byte identical. CPython is the live oracle
//! (no `.expected` fixtures).

use std::path::{Path, PathBuf};
use std::process::Command;

/// The phase spec entries — an explicit allowlist that grows one feature at a
/// time. Each file's compiled stdout must match CPython byte-for-byte.
const PHASE_CORPUS: &[&str] = &[
    "test_hello.py",
    "p2_scalars_print.py",
    "p2_expr.py",
    "p2_control.py",
    "p2_gc_stress.py",
    "p2_funcs.py",
    "test_main.py",
    "p2_bignum.py",
    "p3_numeric.py",
    // Phase 4A — container literals, indexed read/write, len/in, operators.
    "p4_literals.py",
    "p4_subscript.py",
    "p4_operators.py",
    // Phase 4B — general for-loop + iterator protocol + tuple unpacking.
    "p4_for_iter.py",
    "p4_unpack.py",
    // Phase 4C — comprehensions + iteration builtins.
    "p4_comprehensions.py",
    "p4_iter_builtins.py",
    // Phase 4D — focused container methods.
    "p4_methods.py",
    // Phase 4 — cross-feature integration + GC soak (B5/B15).
    "p4_integration.py",
    "p4_gc_stress.py",
    // Phase 5A — core classes: fields, methods, construction.
    "p5_class_basic.py",
    // Phase 5B — inheritance, super(), C3 MRO, virtual dispatch, isinstance.
    "p5_inherit.py",
    // Phase 5C — dunders: arithmetic / comparison / conversion / container.
    "p5_dunder_arith.py",
    "p5_dunder_container.py",
    // Phase 5D — decorators (@staticmethod/@classmethod/@property) + class attrs.
    "p5_decorators.py",
    // Phase 5E — generics: TypeVar / Generic[T] / typed instantiation.
    "p5_generics.py",
    // Phase 5 — class-instance-graph GC soak (uniform-tagged field tracing).
    "p5_gc_stress.py",
    // Phase 6A — closures, lambdas, functions as values.
    "p6_closures.py",
    "p6_lambda_hof.py",
    // Phase 6B — nonlocal / global.
    "p6_nonlocal_global.py",
    // Phase 6C — defaults, keyword args, *args / **kwargs.
    "p6_varargs.py",
    "p6_defaults_kwargs.py",
    // Phase 6D — user decorators (functions).
    "p6_decorators.py",
    // Phase 6E — generators, send/close, generator expressions, GC soak.
    "p6_generators.py",
    "p6_send_close.py",
    "p6_genexpr.py",
    "p6_gc_stress.py",
    // Phase 7A — raise + try/except (builtin exceptions).
    "p7_raise_tryexcept.py",
    "test_multi_except.py",
    // Phase 7B — finally/else, raise-from chaining, instance surface.
    "p7_finally.py",
    "test_traceback.py",
    // Phase 7C — custom exception classes.
    "p7_custom_exc.py",
    // Phase 7D — context managers.
    "p7_with.py",
    "test_exceptions.py",
    // Phase 7E — structural match.
    "p7_match.py",
    "test_match.py",
    // Phase 7 — raise/catch GC soak (shadow-stack unwind + rooted `as e`).
    "p7_gc_stress.py",
];

#[test]
fn phase_corpus_matches_cpython() {
    let pyaot = PathBuf::from(env!("CARGO_BIN_EXE_pyaot"));
    let target_dir = pyaot
        .parent()
        .expect("pyaot binary has a parent target dir")
        .to_path_buf();
    let runtime_lib = ensure_runtime_lib(&target_dir);
    let corpus_dir = workspace_root().join("corpus");
    let out_dir = std::env::temp_dir().join("pyaot_phase1");
    std::fs::create_dir_all(&out_dir).expect("create temp out dir");

    for entry in PHASE_CORPUS {
        let source = corpus_dir.join(entry);
        assert!(
            source.exists(),
            "corpus file {} does not exist",
            source.display()
        );

        let exe = out_dir.join(Path::new(entry).file_stem().unwrap());

        // ── Compile with pyaot. ──
        let compile = Command::new(&pyaot)
            .arg(&source)
            .arg("-o")
            .arg(&exe)
            .arg("--runtime-lib")
            .arg(&runtime_lib)
            .output()
            .expect("failed to spawn pyaot");
        assert!(
            compile.status.success(),
            "pyaot failed to compile {}:\n--- stdout ---\n{}\n--- stderr ---\n{}",
            entry,
            String::from_utf8_lossy(&compile.stdout),
            String::from_utf8_lossy(&compile.stderr),
        );

        // ── Run the compiled executable. ──
        let compiled = Command::new(&exe)
            .output()
            .unwrap_or_else(|e| panic!("failed to run compiled {}: {e}", exe.display()));
        assert!(
            compiled.status.success(),
            "compiled {} exited with failure: {:?}\nstderr:\n{}",
            entry,
            compiled.status,
            String::from_utf8_lossy(&compiled.stderr),
        );

        // ── Run the CPython oracle live. ──
        let oracle = Command::new("python3")
            .arg(&source)
            .output()
            .expect("failed to spawn python3");
        assert!(
            oracle.status.success(),
            "python3 failed on {}:\n{}",
            entry,
            String::from_utf8_lossy(&oracle.stderr),
        );

        // ── Diff stdout byte-for-byte. ──
        assert_eq!(
            String::from_utf8_lossy(&compiled.stdout),
            String::from_utf8_lossy(&oracle.stdout),
            "stdout mismatch for {entry} (pyaot vs CPython)",
        );
    }
}

/// Locate (and build if missing) the runtime staticlib next to the `pyaot`
/// binary, matching the test's build profile. The runtime is **not** a Cargo
/// dependency of the CLI (it's linked from a `.a`), so `cargo test` alone does
/// not produce it — build it here so the gate is self-contained (PITFALLS B9).
fn ensure_runtime_lib(target_dir: &Path) -> PathBuf {
    let lib = target_dir.join("libpyaot_runtime.a");
    if lib.exists() {
        return lib;
    }

    // The profile is the parent dir's name (e.g. `debug` / `release`).
    let profile = target_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("debug");

    let mut cmd = Command::new(env!("CARGO"));
    cmd.arg("build").arg("-p").arg("pyaot-runtime");
    if profile == "release" {
        cmd.arg("--release");
    }
    let build = cmd.output().expect("failed to spawn cargo build for runtime");
    assert!(
        build.status.success(),
        "failed to build pyaot-runtime staticlib:\n{}",
        String::from_utf8_lossy(&build.stderr),
    );
    assert!(
        lib.exists(),
        "runtime staticlib still missing after build: {}",
        lib.display()
    );
    lib
}

/// Workspace root = two levels up from this crate's manifest dir (`crates/cli`).
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("canonicalize workspace root")
}
