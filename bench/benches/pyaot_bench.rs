//! Phase-0 benchmark harness — drives every `bench/py/*.py` source through
//! the pyaot toolchain and records wall-clock time for two metrics:
//!
//!   * `run::<stem>`         — execute a pre-compiled binary (hot-loop perf).
//!   * `end_to_end::<stem>`  — compile + execute (compile+exec pipeline).
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
        let exe = tmp.join(&stem);

        // Pre-compile once for the run-only group.
        compile(&src, &exe);

        // run::<stem> — hot-loop perf, no compile on the hot path.
        let mut run_group = c.benchmark_group(format!("run::{stem}"));
        // Bench binaries may take hundreds of ms each; cap sample count so a
        // full `cargo bench` completes in a reasonable time on CI.
        run_group.sample_size(10);
        run_group.measurement_time(Duration::from_secs(15));
        run_group.bench_function("wall", |b| b.iter(|| run(&exe)));
        run_group.finish();

        // end_to_end::<stem> — compile + execute. Recompiles every sample so
        // regressions in either stage surface.
        let mut e2e_group = c.benchmark_group(format!("end_to_end::{stem}"));
        e2e_group.sample_size(10);
        e2e_group.measurement_time(Duration::from_secs(30));
        e2e_group.bench_function("wall", |b| {
            b.iter(|| {
                compile(&src, &exe);
                run(&exe);
            })
        });
        e2e_group.finish();
    }
}

criterion_group!(benches, drive_bench_suite);
criterion_main!(benches);
