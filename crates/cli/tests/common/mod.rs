//! Shared test utilities for runtime integration tests.

use pyaot::CompileOptions;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Once};
use std::time::Duration;

static BUILD_RUNTIME: Once = Once::new();

/// Maximum time a compiled test binary is allowed to run before being killed.
const EXECUTION_TIMEOUT: Duration = Duration::from_secs(30);

/// Ensure the runtime library is built in release mode (exactly once across all tests).
///
/// Always builds in release mode regardless of the test profile, because the
/// runtime library must be optimized — debug-mode runtime has different overflow
/// behavior and performance characteristics that cause test failures.
fn ensure_runtime_built() {
    BUILD_RUNTIME.call_once(|| {
        let status = Command::new("cargo")
            .args(["build", "-p", "pyaot-runtime", "--release"])
            .status()
            .expect("failed to run cargo build for runtime");
        assert!(
            status.success(),
            "cargo build -p pyaot-runtime --release failed"
        );
    });
}

/// Get the workspace root directory.
pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("failed to canonicalize workspace root")
}

/// Get the path to the runtime library (always release build).
fn runtime_lib_path() -> PathBuf {
    workspace_root().join("target/release/libpyaot_runtime.a")
}

/// Run a command with a timeout. Returns None if the process was killed due to timeout.
fn run_with_timeout(cmd: &mut Command, timeout: Duration) -> Option<Output> {
    let child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn process");

    let child_id = child.id();
    let done = Arc::new(AtomicBool::new(false));
    let done_clone = done.clone();

    // Spawn a watchdog thread that kills the child after timeout
    std::thread::spawn(move || {
        std::thread::sleep(timeout);
        if !done_clone.load(Ordering::SeqCst) {
            // Timeout reached, kill the process via system kill command
            let _ = Command::new("kill")
                .args(["-9", &child_id.to_string()])
                .status();
        }
    });

    let output = child.wait_with_output().expect("failed to wait on child");
    let timed_out = !done.swap(true, Ordering::SeqCst);

    // Check if the process was killed by our watchdog (signal 9 = SIGKILL)
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if output.status.signal() == Some(9) && !timed_out {
            // Process was killed, but we already set done=true, so it wasn't
            // our watchdog. Treat as normal failure.
            return Some(output);
        }
        if output.status.signal() == Some(9) {
            return None;
        }
    }

    Some(output)
}

/// Compile and run a Python test file, optionally checking expected output.
pub fn run_pyaot(test_name: &str, py_file: &Path, expected_output: Option<&str>) {
    ensure_runtime_built();

    let runtime_lib = runtime_lib_path();
    assert!(
        runtime_lib.exists(),
        "Runtime library not found at: {}",
        runtime_lib.display()
    );

    // Create temp dir for this test
    let tmp_dir = std::env::temp_dir().join(format!("pyaot_test_{test_name}"));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).expect("failed to create temp dir");

    let output_bin = tmp_dir.join(test_name);

    // Compile
    let result = pyaot::compile_to_executable(&CompileOptions {
        input: py_file.to_path_buf(),
        output: output_bin.clone(),
        runtime_lib,
        ..Default::default()
    });

    if let Err(e) = &result {
        let _ = std::fs::remove_dir_all(&tmp_dir);
        panic!("Compilation failed for {test_name}: {e:?}");
    }

    // Run the compiled executable with a timeout
    let run_output = run_with_timeout(&mut Command::new(&output_bin), EXECUTION_TIMEOUT);

    let run_output = match run_output {
        Some(output) => output,
        None => {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            panic!(
                "Execution timed out for {test_name} (limit: {}s)",
                EXECUTION_TIMEOUT.as_secs()
            );
        }
    };

    let stdout = String::from_utf8_lossy(&run_output.stdout);
    let stderr = String::from_utf8_lossy(&run_output.stderr);

    if !run_output.status.success() {
        let _ = std::fs::remove_dir_all(&tmp_dir);
        panic!(
            "Execution failed for {test_name} (exit code: {:?}).\nstdout: {stdout}\nstderr: {stderr}",
            run_output.status.code()
        );
    }

    // Check expected output
    if let Some(expected) = expected_output {
        if stdout.trim_end() != expected.trim_end() {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            panic!(
                "Output mismatch for {test_name}.\n\
                 === Expected ===\n{expected}\n\
                 === Got ===\n{stdout}"
            );
        }
    }

    // Cleanup
    let _ = std::fs::remove_dir_all(&tmp_dir);
}
