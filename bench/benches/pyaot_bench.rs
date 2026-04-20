//! Phase-0 benchmark harness — drives every `bench/py/*.py` source through
//! the pyaot toolchain and records wall-clock time for three metrics:
//!
//!   * `compile::<stem>`     — compiler + linker throughput only.
//!   * `run::<stem>`         — execute a pre-compiled binary (hot-loop perf).
//!   * `fresh_launch::<stem>` — compile + immediate first launch of the
//!                              freshly linked binary.
//!
//! Binary size is recorded once per source at baseline time and written into
//! `bench/BASELINE.md` out-of-band — Criterion doesn't have a natural
//! channel for that. Max RSS is captured by wrapping the binary with
//! `/usr/bin/time -l` (macOS) or `/usr/bin/time -v` (Linux) during baseline
//! runs; see `bench/README.md` for the recipe. Keeping those two metrics
//! out of Criterion keeps the harness portable.
//!
//! The harness assumes the release pyaot binary and runtime artifacts have
//! been built — it does NOT invoke cargo. Run
//!
//!     cargo build --workspace --release
//!     cargo bench -p pyaot-bench
//!
//! before expecting meaningful numbers.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, Criterion};
use pyaot_bench::{cleanup_output_artifacts, stable_output_path, unique_output_path};

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is <repo>/bench; walk one level up.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("bench/ has no parent")
        .to_path_buf()
}

fn pyaot_binary() -> PathBuf {
    let exe = if cfg!(windows) { "pyaot.exe" } else { "pyaot" };
    repo_root().join("target").join("release").join(exe)
}

fn runtime_lib() -> PathBuf {
    repo_root()
        .join("target")
        .join("release")
        .join("libpyaot_runtime.a")
}

fn py_dir() -> PathBuf {
    repo_root().join("bench").join("py")
}

fn bench_out_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("pyaot-bench");
    std::fs::create_dir_all(&dir).expect("could not create bench out dir");
    dir
}

fn compile(src: &Path, out: &Path) {
    let bin = pyaot_binary();
    assert!(
        bin.exists(),
        "pyaot release binary not found at {bin:?} — run `cargo build --workspace --release` first"
    );
    let rt = runtime_lib();
    assert!(
        rt.exists(),
        "pyaot runtime library not found at {rt:?} — run `cargo build --workspace --release` first"
    );
    let status = Command::new(&bin)
        .arg(src)
        .arg("-o")
        .arg(out)
        .arg("--runtime-lib")
        .arg(&rt)
        .status()
        .expect("failed to spawn pyaot");
    assert!(status.success(), "pyaot failed to compile {src:?}");
}

fn run(exe: &Path) {
    let status = Command::new(exe)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("failed to spawn benchmark binary");
    assert!(status.success(), "benchmark binary {exe:?} exited non-zero");
}

fn collect_sources() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let dir = py_dir();
    for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {dir:?}: {e}")) {
        let p = entry.expect("dir entry").path();
        if p.extension().and_then(|e| e.to_str()) == Some("py") {
            out.push(p);
        }
    }
    out.sort();
    out
}

fn drive_bench_suite(c: &mut Criterion) {
    let tmp = bench_out_dir();
    for src in collect_sources() {
        let stem = src.file_stem().unwrap().to_string_lossy().to_string();
        let run_exe = stable_output_path(&tmp, &stem, "run");

        // run::<stem> — hot-loop perf, no compile on the hot path.
        // Compile lazily here rather than once up front for every source so a
        // filtered `cargo bench ... strings` run doesn't precompile unrelated
        // programs and perturb the machine before measurement starts.
        let run_src = src.clone();
        let run_exe_clone = run_exe.clone();
        let mut run_group = c.benchmark_group(format!("run::{stem}"));
        run_group.sample_size(10);
        run_group.measurement_time(Duration::from_secs(15));
        run_group.bench_function("wall", move |b| {
            compile(&run_src, &run_exe_clone);
            b.iter(|| run(&run_exe_clone));
            cleanup_output_artifacts(&run_exe_clone);
        });
        run_group.finish();

        // compile::<stem> — compiler + linker throughput only. Use a unique
        // output path every sample so the measurement reflects compile work
        // rather than any platform-specific "relaunch a freshly replaced
        // binary at the same path" behavior.
        let compile_src = src.clone();
        let compile_tmp = tmp.clone();
        let compile_stem = stem.clone();
        let mut compile_sample = 0_u64;
        let mut compile_group = c.benchmark_group(format!("compile::{stem}"));
        compile_group.sample_size(10);
        compile_group.measurement_time(Duration::from_secs(15));
        compile_group.bench_function("wall", move |b| {
            b.iter(|| {
                let out =
                    unique_output_path(&compile_tmp, &compile_stem, "compile", compile_sample);
                compile_sample += 1;
                compile(&compile_src, &out);
                cleanup_output_artifacts(&out);
            })
        });
        compile_group.finish();

        // fresh_launch::<stem> — compile and immediately launch the freshly
        // linked executable. On macOS this captures launch validation and
        // other path-sensitive first-run effects, so it is useful as a
        // diagnostic metric but should not be treated as pure compiler
        // throughput.
        let launch_src = src.clone();
        let launch_tmp = tmp.clone();
        let launch_stem = stem.clone();
        let mut launch_sample = 0_u64;
        let mut launch_group = c.benchmark_group(format!("fresh_launch::{stem}"));
        launch_group.sample_size(10);
        launch_group.measurement_time(Duration::from_secs(30));
        launch_group.bench_function("wall", move |b| {
            b.iter(|| {
                let out =
                    unique_output_path(&launch_tmp, &launch_stem, "fresh-launch", launch_sample);
                launch_sample += 1;
                compile(&launch_src, &out);
                run(&out);
                cleanup_output_artifacts(&out);
            })
        });
        launch_group.finish();
    }
}

criterion_group!(benches, drive_bench_suite);
criterion_main!(benches);
