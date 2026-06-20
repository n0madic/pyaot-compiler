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
const SELF_CHECKING: &[&str] = &[
    "test_stdlib_time.py",
    "test_file_io_core.py",
    "test_stdlib_subprocess.py",
];

/// The gate — one consolidated `test_*.py` file per large feature category.
/// Each file's compiled stdout must match CPython byte-for-byte (except the
/// [`SELF_CHECKING`] few). Every check inside is an `assert` (so a wrong result
/// fails the run on BOTH pyaot and CPython); the only intentional `print`s are
/// `test_print_output.py` (where stdout IS the feature), `test_traceback.py`,
/// and each file's single final "...passed!" marker. The former per-feature
/// point files (`pN_*.py`) were folded into these categories — see the fold
/// notes on each entry.
const PHASE_CORPUS: &[&str] = &[
    // ── Smoke / entry / print() formatting ──
    // test_main.py is the minimal canary (compile→link→run→stdout) AND covers the
    // `__name__ == "__main__"` entry guard; the old `print("hello")` test_hello.py
    // was a strict subset of test_print_output.py and was dropped.
    "test_main.py",
    // print() sep/end/flush/formatting — the one suite where stdout IS the
    // feature under test (kept print-based by design).
    "test_print_output.py",
    // ── Core language ──
    // Scalars/expressions/operators/bignum/numeric tower + raw-int loop & interproc
    // specialization (folds p2_bignum/expr/scalars_print, p3_numeric,
    // p3c_raw_int_loops/interproc_raw, p4_literals/operators/subscript, p16/p44).
    "test_core_types.py",
    // if/elif/while/for + `is`/`is not` identity + `del` + walrus `:=`
    // (folds p2_control, p11_is_identity, p12_del, p26_walrus).
    "test_control_flow.py",
    // Closures/lambdas/varargs/kwargs/defaults/decorators/spread/value-call
    // (folds p2_funcs, p6_closures/lambda_hof/nonlocal_global/varargs/
    // defaults_kwargs/decorators, p10_kwargs_evalorder/methods, p13_spread,
    // p36/p37/p38/p39/p40/p41, decorator_factory, class_decorators).
    "test_functions.py",
    // Generators / send / close / generator expressions
    // (folds p6_generators, p6_send_close, p6_genexpr).
    "test_generators.py",
    // ── OOP / types ──
    // Classes: fields/methods/inherit/C3-MRO/dunders/iterator-protocol/gradual
    // dispatch/field inference/heap & numeric-tower seams (folds p5_class_basic/
    // inherit/mro_join/dunder_arith/dunder_container/decorators, p27_matmul,
    // p42_iter_protocol, p43_hetero_tuple_iter, p45/p46/p47/p48, p8h_dyn_attr,
    // gradual_methods, b10_field_inference).
    "test_classes.py",
    // Generics + type system + dead-code narrowing (merges test_types_system.py
    // and test_dead_code_warnings.py; folds p5_generics).
    "test_generics.py",
    // ── Iteration & collections ──
    // for/comprehensions/iter-builtins/unpack/reduce/map/filter/tuple-cmp +
    // cross-feature integration + comprehension outermost-iterable scope (folds
    // p4_for_iter/unpack/comprehensions/iter_builtins/integration,
    // p14_nested_unpack, p22/p23/p24/p25/p28, test_review_fixes§comp-scope).
    "test_iteration.py",
    // list/tuple + dict/set/bytes + Counter/defaultdict/deque/OrderedDict +
    // container methods (merges test_collections_list_tuple.py and
    // test_collections_dict_set_bytes.py; folds p4_methods, p15_tuple_slice_slot,
    // p21_container_methods, p35_counter, p51_container_aug_ops).
    "test_collections.py",
    // ── Builtins & strings ──
    // Scalar/introspection builtins, zip(N≥3), int methods, conversions,
    // isinstance-tuple, type(), builtins-as-values (folds p17_type_builtin,
    // p18_scalar_builtins, p30/p31/p32/p33/p34, p10_kwargs_builtins,
    // builtin_first_class).
    "test_builtins.py",
    // str/bytes methods + unicode predicates + the format mini-language (merges
    // test_format_spec.py; folds p19_str_methods, p20_bytes_methods,
    // p49_str_method_args, p50_unicode_predicates, p8h_unicode, p29_format).
    "test_strings.py",
    // ── Exceptions & structural match ──
    // raise/try/except/finally/custom/with/multi-except + runtime-unpack arity
    // ValueError + list out-of-range IndexError + instance<op>immediate
    // NotImplemented→TypeError (folds p7_raise_tryexcept, p7_finally,
    // p7_custom_exc, p7_with, multi_except, test_review_fixes).
    "test_exceptions.py",
    // Real tracebacks — output-format suite, standalone (line markers + lazy
    // PC−1 resolution); kept print-based since the traceback text IS the output.
    "test_traceback.py",
    // Structural `match` — literals/class-patterns/capture/guards/OR/sequence/
    // mapping (folds p7_match).
    "test_match.py",
    // ── Cross-cutting language gaps + GC soak ──
    // f-string specs/slicing/sum/comprehension-element inference/file iteration/
    // os.environ/module-level lambda defaults (folds p8e_language, p8h_lang,
    // p8h_comp_elem, p8h_sum).
    "test_language.py",
    // Consolidated GC-stress soak — allocate-heavy loops across call/exception/
    // generator/zip/closure boundaries (folds p2/p4/p5/p6/p7_gc_stress, the two
    // p9_*_gc_stress, gc_simple). Run under `RUSTFLAGS="--cfg gc_stress_test"` to
    // surface a missed root as a use-after-free.
    "test_gc_stress.py",
    // ── Modules / imports / annotations ──
    "test_global_scoping.py",
    "test_import.py",
    "test_packages.py",
    "test_reexport.py",
    "test_future_annotations.py",
    // ── Stdlib (per module; fixtures, self-checking, and network modes differ) ──
    "test_stdlib_math.py",
    "test_stdlib_random.py",
    "test_stdlib_sys.py",
    "test_stdlib_time.py", // self-checking — live timestamps
    "test_stdlib_re.py",
    "test_stdlib_json.py",
    "test_stdlib_os.py",
    "test_stdlib_subprocess.py", // self-checking — subprocess stdout type differs
    "test_stdlib_itertools.py",
    "test_stdlib_urllib_core.py",
    // Cross-module stdlib edge-case parity: seam safety (join/dict.get/json/None),
    // urlencode/posixpath/quote/json edges, slice step=0, and checked Dyn→raw
    // unbox at math/number raw-ABI boundaries (folds p8g_seam_safety,
    // p8h_stdlib_edges, p8h_checked_unbox, p8h_checked_unbox2).
    "test_stdlib_edges.py",
    // ── File I/O ──
    "test_file_io.py",
    "test_file_io_core.py", // self-checking — writes /tmp paths
    // ── Capstone ──
    // Karpathy's microgpt on real stdlib — byte-exact, exercises nearly every
    // front-half feature at once. Kept as its own standalone case.
    "microgpt.py",
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

/// Locate (and rebuild) the runtime staticlib next to the `pyaot` binary,
/// matching the test's build profile. The runtime is **not** a Cargo
/// dependency of the CLI (it's linked from a `.a`), so `cargo test` alone does
/// not produce it — build it here so the gate is self-contained (PITFALLS B9).
///
/// `cargo build` runs UNCONDITIONALLY (it is incremental — a no-op when the
/// runtime sources are unchanged, a fraction of a second). Returning a stale
/// `.a` just because it exists silently links yesterday's runtime: a runtime
/// fix would not take effect and could SIGSEGV at link-clean compile time
/// (this bit the Phase-10 `rt_sorted` ABI change).
fn ensure_runtime_lib(target_dir: &Path) -> PathBuf {
    let lib = target_dir.join("libpyaot_runtime.a");

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
    let build = cmd
        .output()
        .expect("failed to spawn cargo build for runtime");
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
