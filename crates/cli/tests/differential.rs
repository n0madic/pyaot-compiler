//! Differential harness — the Phase-1 gate.
//!
//! For each file in [`PHASE_CORPUS`] (an explicit **allowlist**, NOT a glob, so
//! the full-feature corpus files cannot break this phase's gate): compile it with
//! `pyaot`, run the resulting executable, run the same file under `python3`, and
//! compare the two stdouts per the entry's [`DiffMode`]. CPython is the live
//! oracle (no `.expected` fixtures).

use std::path::{Path, PathBuf};
use std::process::Command;

/// Entries whose intermediate output is inherently run-dependent (live
/// timestamps), compared in self-checking mode instead of byte-diff: both
/// runs must exit 0 and end with the same final line — the file's own
/// "…passed!" marker, which only prints when every internal assert held.
const SELF_CHECKING: &[&str] =
    &["test_stdlib_time.py", "test_file_io_core.py", "test_stdlib_subprocess.py"];

/// The phase spec entries — an explicit allowlist that grows one feature at a
/// time. Each file's compiled stdout must match CPython byte-for-byte
/// (except the [`SELF_CHECKING`] few).
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
    // Phase 8A — module-level global scoping (annotated globals stay typed even
    // when written inside functions). No imports — exercises the global
    // infrastructure the import machinery promotes module bindings onto.
    "test_global_scoping.py",
    // Phase 8A — multi-module: `import` / `from … import` (incl. relative),
    // packages, cross-module functions / classes / constants / generators. The
    // `math_utils.py` / `genmod.py` / `mypackage/**` trees are fixtures (search
    // path = the source's directory), never gate entries themselves.
    "test_import.py",
    "test_packages.py",
    // Phase 8A — package re-export: a package `__init__.py` that publishes names
    // it imported from a submodule (`from .circle import Circle`). The canonical
    // facade — `from shapes import Circle` and `import shapes; shapes.Circle`.
    "test_reexport.py",
    // Phase 8B — the descriptor-driven stdlib `CallRuntime` path: math (raw
    // f64/i64 ABI + constants), random (CPython-exact MT19937, kwargs,
    // pass_arg_count, absent-optional sentinel), sys (attr getters, argv/path
    // singletons), time (struct_time fields; self-checking — live timestamps).
    "test_stdlib_math.py",
    "test_stdlib_random.py",
    "test_stdlib_sys.py",
    "test_stdlib_time.py",
    // Phase 8C — stdlib object types: re/Match methods (group/span via the
    // object-type registry), json (dumps/loads + dump/load to a File), and
    // File I/O (open/read/write/with/modes/encoding/iteration, io.StringIO/
    // BytesIO). `test_file_io_core.py` is self-checking — it writes /tmp paths.
    "test_stdlib_re.py",
    "test_stdlib_json.py",
    "test_file_io_core.py",
    // Phase 8D — os / os.path (submodule-chain folding + variadic join), the
    // `os.environ` dict attr, subprocess.run (CompletedProcess fields;
    // self-checking — subprocess stdout is bytes in CPython, str here), and
    // urllib (parse + ParseResult/Request fields, plus the urllib.error
    // exception hierarchy raised/caught by id and by builtin parent). The
    // network urlopen/urlretrieve sections are excluded (non-deterministic).
    "test_stdlib_os.py",
    "test_stdlib_subprocess.py",
    "test_stdlib_urllib_core.py",
    // Phase 8E — language gaps for real scripts: f-string format specs, slicing
    // (list/str/tuple, negative/stepped/open-ended), str.join / list.index, the
    // tuple `()` parameter default + `__slots__`, and the cross-function
    // return-type inference that keeps unannotated dunder/method results typed.
    "p8e_language.py",
    // Phase 8F — the capstone: Karpathy's microgpt (autograd `Value` with 12
    // dunders, multi-head attention, Adam training, temperature sampling) on real
    // stdlib (math.log/exp, random.gauss/shuffle/choices — MT19937 + libm match
    // CPython bit-for-bit). A small model config keeps it byte-exact yet fast
    // under the debug runtime. Exercises nearly every front-half feature at once.
    "microgpt.py",
    // Phase 8 seam-safety regressions: the stdlib/container seam used to pass a
    // mismatched heap shape (str/tuple/generator to `join`, a heap-valued
    // `dict.get` miss, a str-keyed `json.loads` subscript) or a bare None to the
    // frozen runtime, which dereferenced it without a guard — SEGVs + silent
    // wrong values. Also `list.index(missing)` now raises `ValueError`.
    "p8g_seam_safety.py",
];

/// Network-dependent entries, run (self-checking) ONLY when `PYAOT_NET_TESTS` is
/// set — `test_stdlib_urllib.py` exercises the live `urlopen`/`urlretrieve` paths
/// its `_core` sibling excludes. It is offline-safe: every network section is
/// wrapped in `try/except IOError`, so a connection failure skips to the same
/// final "All urllib tests passed!" line on both pyaot and CPython.
const NET_TESTS: &[&str] = &["test_stdlib_urllib.py"];

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

    // The default gate is PHASE_CORPUS; `PYAOT_NET_TESTS` adds the live-network
    // entries (self-checking).
    let mut entries: Vec<&str> = PHASE_CORPUS.to_vec();
    if std::env::var_os("PYAOT_NET_TESTS").is_some() {
        entries.extend_from_slice(NET_TESTS);
    }

    for entry in &entries {
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

        // ── Compare stdout: byte-for-byte, or final-line for self-checking
        // entries (both already exited 0 above). ──
        if SELF_CHECKING.contains(entry) || NET_TESTS.contains(entry) {
            let last = |out: &[u8]| -> String {
                String::from_utf8_lossy(out)
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or_default()
                    .to_string()
            };
            assert_eq!(
                last(&compiled.stdout),
                last(&oracle.stdout),
                "final self-check line mismatch for {entry} (pyaot vs CPython)",
            );
        } else {
            assert_eq!(
                String::from_utf8_lossy(&compiled.stdout),
                String::from_utf8_lossy(&oracle.stdout),
                "stdout mismatch for {entry} (pyaot vs CPython)",
            );
        }
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
