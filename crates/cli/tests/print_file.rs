//! `print(..., file=sys.stdout / sys.stderr)` gate. The main differential
//! harness compares stdout ONLY, so it can prove a `file=sys.stderr` line never
//! leaks INTO stdout — but not that the stderr stream itself is byte-correct.
//! This standalone test fills that gap: it compiles a fixture exercising every
//! print kind on both streams, runs it and `python3`, and compares BOTH stdout
//! and stderr byte-for-byte. The fixture exits 0, so CPython is a live oracle
//! (unlike the traceback gate, which must hardcode its expectation).

use std::path::{Path, PathBuf};
use std::process::Command;

const FIXTURE: &str = r#"import sys

# Default and explicit stdout are identical.
print("stdout default")
print("stdout explicit", file=sys.stdout)

# A single stderr line, then back to stdout (the target is sticky, so lowering
# must restore stdout after the redirected line).
print("stderr line", file=sys.stderr)
print("after stderr")

# Every print KIND must honor the target: ints, floats, bools, None, the
# separator, the custom end, and nested container repr — not just strings.
print("e", 1, 2.5, True, None, sep="|", end="!\n", file=sys.stderr)
print([1, 2, 3], (4, 5), {"k": "v"}, file=sys.stderr)
print("nested", [[1], [2]], file=sys.stderr)

class P:
    def __init__(self, n: int):
        self.n = n

    def __str__(self) -> str:
        return "P<" + str(self.n) + ">"

print(P(9), file=sys.stderr)

# A side-effecting argument's own output goes to the CURRENT (stdout) target,
# evaluated BEFORE the line is redirected to stderr.
def loud(x: int) -> int:
    print("eval", x)
    return x

print(loud(3), file=sys.stderr)

# flush= on both streams and with end="" (the case where the line buffer would
# otherwise hold the bytes): content is identical, flush only affects timing.
print("flush stdout", flush=True)
print("flush stderr", file=sys.stderr, flush=True)
print("partial", end="", flush=True)
print(" + rest", flush=False)

print("last stdout")
"#;

#[test]
fn print_file_redirects_both_streams() {
    let pyaot = PathBuf::from(env!("CARGO_BIN_EXE_pyaot"));
    let target_dir = pyaot.parent().expect("target dir").to_path_buf();
    let runtime_lib = ensure_runtime_lib(&target_dir);

    let out_dir = std::env::temp_dir().join("pyaot_print_file_test");
    std::fs::create_dir_all(&out_dir).expect("create temp out dir");
    let source = out_dir.join("print_file_fixture.py");
    std::fs::write(&source, FIXTURE).expect("write fixture");
    let exe = out_dir.join("print_file_fixture");

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
        "pyaot failed to compile the print-file fixture:\n--- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&compile.stdout),
        String::from_utf8_lossy(&compile.stderr),
    );

    let run = Command::new(&exe).output().expect("failed to run fixture");
    assert!(
        run.status.success(),
        "compiled fixture exited nonzero:\nstderr:\n{}",
        String::from_utf8_lossy(&run.stderr),
    );

    let oracle = Command::new("python3")
        .arg(&source)
        .output()
        .expect("failed to spawn python3");
    assert!(
        oracle.status.success(),
        "python3 failed on the fixture:\n{}",
        String::from_utf8_lossy(&oracle.stderr),
    );

    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&oracle.stdout),
        "stdout mismatch (pyaot vs CPython)",
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stderr),
        String::from_utf8_lossy(&oracle.stderr),
        "stderr mismatch (pyaot vs CPython)",
    );
}

/// Locate (and build if missing) the runtime staticlib next to the `pyaot`
/// binary, matching the test's build profile (mirrors the differential gate).
fn ensure_runtime_lib(target_dir: &Path) -> PathBuf {
    let lib = target_dir.join("libpyaot_runtime.a");
    if lib.exists() {
        return lib;
    }
    let profile = target_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("debug");
    let mut cmd = Command::new(env!("CARGO"));
    cmd.arg("build").arg("-p").arg("pyaot-runtime");
    if profile == "release" {
        cmd.arg("--release");
    }
    let build = cmd.output().expect("failed to spawn cargo build");
    assert!(
        build.status.success(),
        "failed to build pyaot-runtime staticlib:\n{}",
        String::from_utf8_lossy(&build.stderr),
    );
    lib
}
