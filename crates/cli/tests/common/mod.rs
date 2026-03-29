//! Shared test utilities for runtime integration tests.

use pyaot::CompileOptions;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Once, OnceLock};
use std::time::Duration;

static BUILD_RUNTIME: Once = Once::new();

/// Serialize compilations to avoid thread-safety issues in Cranelift's
/// ObjectModule when multiple compilations run in the same process.
/// The compiled executables still run in parallel.
static COMPILE_MUTEX: Mutex<()> = Mutex::new(());

/// Maximum time a compiled test binary is allowed to run before being killed.
const EXECUTION_TIMEOUT: Duration = Duration::from_secs(30);

/// Ensure the runtime library is built in release mode (exactly once across all tests).
///
/// Always uses release mode because the runtime is designed for optimized operation —
/// debug mode has known elem_tag mismatches in map/filter/tuple operations that
/// cause GC to misidentify raw int/bool values as heap pointers, leading to
/// nondeterministic failures under GC pressure.
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

/// Declarative rule for a known acceptable line-level difference.
///
/// When a line differs between PyAOT and CPython, the rule matches if:
/// - the PyAOT line contains `pyaot_contains`
/// - the CPython line contains `cpython_contains`
///
/// Attached as metadata to individual tests via the `diffs:` syntax in `runtime_cases!`.
pub struct AllowedDiff {
    pub pyaot_contains: &'static str,
    pub cpython_contains: &'static str,
    pub reason: &'static str,
}

/// Compile and run a Python test file, optionally checking expected output.
pub fn run_pyaot(
    test_name: &str,
    py_file: &Path,
    expected_output: Option<&str>,
    allowed_diffs: &[AllowedDiff],
) {
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

    // Compile (serialized to avoid Cranelift thread-safety issues)
    let result = {
        let _lock = COMPILE_MUTEX.lock().expect("compile mutex poisoned");
        pyaot::compile_to_executable(&CompileOptions {
            input: py_file.to_path_buf(),
            output: output_bin.clone(),
            runtime_lib,
            ..Default::default()
        })
    };

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

    // CPython differential check (only when PYAOT_DIFF_TEST=1)
    if should_run_cpython_diff() && !is_skipped_for_diff(test_name) {
        match run_in_cpython(py_file) {
            Ok(cpython_stdout) => {
                let pyaot_normalized = normalize_output(&stdout);
                let cpython_normalized = normalize_output(&cpython_stdout);
                if pyaot_normalized != cpython_normalized {
                    check_diff_with_allowed(
                        test_name,
                        &pyaot_normalized,
                        &cpython_normalized,
                        allowed_diffs,
                        &tmp_dir,
                    );
                }
            }
            Err(reason) => {
                eprintln!("WARNING: CPython diff skipped for {test_name}: {reason}");
            }
        }
    }

    // Cleanup
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

// ---------------------------------------------------------------------------
// CPython differential testing
// ---------------------------------------------------------------------------

/// Tests where CPython comparison is entirely impossible (output is fundamentally
/// non-deterministic or environment-dependent throughout).
const CPYTHON_DIFF_SKIP: &[&str] = &[
    "runtime_stdlib_time",       // timestamps, sleep durations permeate output
    "runtime_stdlib_urllib",     // network access, responses vary
    "runtime_file_io",           // temp file paths, timing-dependent
    "runtime_stdlib_subprocess", // str vs bytes capture differences
];

/// Check if CPython differential testing is enabled via environment variable.
fn should_run_cpython_diff() -> bool {
    std::env::var("PYAOT_DIFF_TEST").is_ok_and(|v| v == "1" || v == "true")
}

/// Check if a test is on the skip list for differential testing.
fn is_skipped_for_diff(test_name: &str) -> bool {
    CPYTHON_DIFF_SKIP.contains(&test_name)
}

/// Check whether a specific line difference is allowed by any of the test's rules.
fn is_diff_allowed(
    allowed_diffs: &[AllowedDiff],
    pyaot_line: &str,
    cpython_line: &str,
) -> Option<&'static str> {
    allowed_diffs.iter().find_map(|rule| {
        if pyaot_line.contains(rule.pyaot_contains) && cpython_line.contains(rule.cpython_contains)
        {
            Some(rule.reason)
        } else {
            None
        }
    })
}

/// Compare outputs line by line, tolerating known allowed differences.
/// Panics only if there are unexpected (non-allowed) differences.
fn check_diff_with_allowed(
    test_name: &str,
    pyaot_output: &str,
    cpython_output: &str,
    allowed_diffs: &[AllowedDiff],
    tmp_dir: &Path,
) {
    let pyaot_lines: Vec<&str> = pyaot_output.lines().collect();
    let cpython_lines: Vec<&str> = cpython_output.lines().collect();
    let max_lines = pyaot_lines.len().max(cpython_lines.len());

    let mut unexpected = Vec::new();
    let mut tolerated = Vec::new();

    for i in 0..max_lines {
        let pyaot_line = pyaot_lines.get(i).copied().unwrap_or("<missing>");
        let cpython_line = cpython_lines.get(i).copied().unwrap_or("<missing>");

        if pyaot_line == cpython_line {
            continue;
        }

        if let Some(reason) = is_diff_allowed(allowed_diffs, pyaot_line, cpython_line) {
            tolerated.push((i + 1, reason));
        } else {
            unexpected.push((i + 1, pyaot_line, cpython_line));
        }
    }

    // Line count mismatch is always unexpected
    if pyaot_lines.len() != cpython_lines.len() && unexpected.is_empty() {
        // All line-level diffs were tolerated but line counts differ —
        // this means one output has extra trailing lines
        let shorter = pyaot_lines.len().min(cpython_lines.len());
        for i in shorter..max_lines {
            let pyaot_line = pyaot_lines.get(i).copied().unwrap_or("<missing>");
            let cpython_line = cpython_lines.get(i).copied().unwrap_or("<missing>");
            unexpected.push((i + 1, pyaot_line, cpython_line));
        }
    }

    if unexpected.is_empty() {
        if !tolerated.is_empty() {
            eprintln!(
                "INFO: {test_name}: {} tolerated diff(s): {}",
                tolerated.len(),
                tolerated
                    .iter()
                    .map(|(line, reason)| format!("line {line} ({reason})"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        return;
    }

    // Build failure report
    let mut diff_report = String::new();
    for (line_num, pyaot_line, cpython_line) in &unexpected {
        diff_report.push_str(&format!(
            "  line {line_num}: pyaot={pyaot_line:?} cpython={cpython_line:?}\n",
        ));
    }
    if !tolerated.is_empty() {
        diff_report.push_str(&format!(
            "  ({} other diff(s) tolerated as known divergences)\n",
            tolerated.len()
        ));
    }

    let _ = std::fs::remove_dir_all(tmp_dir);
    panic!(
        "CPython differential test FAILED for {test_name}.\n\
         {} unexpected difference(s):\n{diff_report}\n\
         === PyAOT output ({} lines) ===\n{pyaot_output}\n\
         === CPython output ({} lines) ===\n{cpython_output}",
        unexpected.len(),
        pyaot_lines.len(),
        cpython_lines.len()
    );
}

/// Cached Python 3 interpreter path.
static PYTHON_PATH: OnceLock<Option<String>> = OnceLock::new();

/// Find a working Python 3 interpreter, cached across all tests.
fn find_python() -> Option<String> {
    PYTHON_PATH
        .get_or_init(|| {
            for candidate in &["python3", "python"] {
                if let Ok(output) = Command::new(candidate).args(["--version"]).output() {
                    if output.status.success() {
                        let version = String::from_utf8_lossy(&output.stdout);
                        // Some pythons print version to stderr
                        let version_str = if version.contains("Python 3") {
                            version.to_string()
                        } else {
                            String::from_utf8_lossy(&output.stderr).to_string()
                        };
                        if version_str.contains("Python 3") {
                            return Some(candidate.to_string());
                        }
                    }
                }
            }
            None
        })
        .clone()
}

/// Run a Python file in CPython and capture its stdout.
fn run_in_cpython(py_file: &Path) -> Result<String, String> {
    let python = find_python().ok_or_else(|| "python3 not found in PATH".to_string())?;

    // Set PYTHONPATH to the directory containing the .py file
    // so that relative imports (math_utils, mypackage) resolve correctly.
    let py_dir = py_file
        .parent()
        .unwrap_or(Path::new("."))
        .to_string_lossy()
        .to_string();

    let output = run_with_timeout(
        Command::new(&python)
            .arg(py_file)
            .env("PYTHONPATH", &py_dir),
        EXECUTION_TIMEOUT,
    );

    match output {
        None => Err("CPython execution timed out".to_string()),
        Some(out) => {
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                Err(format!("CPython execution failed: {stderr}"))
            } else {
                Ok(String::from_utf8_lossy(&out.stdout).to_string())
            }
        }
    }
}

/// Normalize output for reliable comparison.
fn normalize_output(output: &str) -> String {
    output
        .lines()
        .map(|line| line.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string()
}
