//! Phase-0 benchmark harness for the pyaot compiler.
//!
//! This crate hosts `cargo bench` targets; it exports no public API. The
//! benchmark sources live under `bench/py/` and the harness that drives them
//! is in `bench/benches/pyaot_bench.rs`. See `bench/README.md` for how to
//! run the suite and compare against the committed baseline in
//! `bench/BASELINE.md`.

#![forbid(unsafe_code)]

use std::fs;
use std::path::{Path, PathBuf};

/// Stable output path used for metrics that intentionally reuse one compiled
/// executable, e.g. the hot `run::<stem>` benchmark.
pub fn stable_output_path(base_dir: &Path, stem: &str, metric: &str) -> PathBuf {
    base_dir.join(format!("{stem}-{metric}"))
}

/// Unique output path used for metrics that must avoid path reuse effects,
/// e.g. compile throughput and fresh-launch measurements.
pub fn unique_output_path(base_dir: &Path, stem: &str, metric: &str, sample: u64) -> PathBuf {
    base_dir.join(format!("{stem}-{metric}-{sample}"))
}

/// Best-effort cleanup for benchmark outputs.
///
/// Bench runs create many short-lived executables; deleting them keeps the
/// temp directory bounded and avoids cross-sample interference from stale
/// files. Missing files are fine.
pub fn cleanup_output_artifacts(output: &Path) {
    let _ = fs::remove_file(output);
    #[cfg(target_os = "macos")]
    {
        let _ = fs::remove_dir_all(output.with_extension("dSYM"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn stable_output_path_uses_metric_suffix() {
        let path = stable_output_path(Path::new("/tmp/pyaot-bench"), "strings", "run");
        assert_eq!(path, PathBuf::from("/tmp/pyaot-bench/strings-run"));
    }

    #[test]
    fn unique_output_path_changes_with_sample_id() {
        let base = Path::new("/tmp/pyaot-bench");
        let a = unique_output_path(base, "strings", "compile", 1);
        let b = unique_output_path(base, "strings", "compile", 2);
        assert_ne!(a, b);
        assert_eq!(a, PathBuf::from("/tmp/pyaot-bench/strings-compile-1"));
        assert_eq!(b, PathBuf::from("/tmp/pyaot-bench/strings-compile-2"));
    }

    #[test]
    fn cleanup_output_artifacts_removes_file_and_tolerates_missing() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time went backwards")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("pyaot-bench-cleanup-{unique}"));
        fs::write(&path, b"bench").expect("write temp bench file");
        assert!(path.exists());

        cleanup_output_artifacts(&path);
        assert!(!path.exists());

        cleanup_output_artifacts(&path);
        assert!(!path.exists());
    }
}
