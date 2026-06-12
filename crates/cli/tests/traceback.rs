//! Real-traceback gate: an UNHANDLED exception prints a CPython-format
//! frame list (file / line / function, outermost first) to stderr and exits
//! nonzero. This cannot live in the differential corpus (the gate there
//! requires exit 0 on both sides), so the expected frames are asserted
//! directly here.
//!
//! Compiled at `--opt-level none` so inlining cannot collapse the frames —
//! frame fidelity under optimization is deliberately weaker (inlined callees
//! merge into their caller, keeping the innermost line).

use std::path::{Path, PathBuf};
use std::process::Command;

const FIXTURE: &str = r#"def divide(a: int, b: int) -> int:
    return a // b

def caller() -> int:
    x: int = 5
    return divide(10, 0)

print("before")
caller()
"#;

#[test]
fn unhandled_exception_prints_real_traceback() {
    let pyaot = PathBuf::from(env!("CARGO_BIN_EXE_pyaot"));
    let target_dir = pyaot.parent().expect("target dir").to_path_buf();
    let runtime_lib = ensure_runtime_lib(&target_dir);

    let out_dir = std::env::temp_dir().join("pyaot_tb_test");
    std::fs::create_dir_all(&out_dir).expect("create temp out dir");
    let source = out_dir.join("tb_fixture.py");
    std::fs::write(&source, FIXTURE).expect("write fixture");
    let exe = out_dir.join("tb_fixture");

    let compile = Command::new(&pyaot)
        .arg(&source)
        .arg("-o")
        .arg(&exe)
        .arg("--opt-level")
        .arg("none")
        .arg("--runtime-lib")
        .arg(&runtime_lib)
        .output()
        .expect("failed to spawn pyaot");
    assert!(
        compile.status.success(),
        "pyaot failed to compile the traceback fixture:\n{}",
        String::from_utf8_lossy(&compile.stderr),
    );

    let run = Command::new(&exe).output().expect("failed to run fixture");
    assert!(
        !run.status.success(),
        "an unhandled exception must exit nonzero"
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "before\n",
        "stdout up to the raise must be intact"
    );

    let stderr = String::from_utf8_lossy(&run.stderr);
    let file = source.display().to_string();
    let expected = [
        "Traceback (most recent call last):".to_string(),
        format!("  File \"{file}\", line 9, in <module>"),
        format!("  File \"{file}\", line 6, in caller"),
        format!("  File \"{file}\", line 2, in divide"),
    ];
    let lines: Vec<&str> = stderr.lines().collect();
    let start = lines
        .iter()
        .position(|l| *l == expected[0])
        .unwrap_or_else(|| panic!("no traceback header in stderr:\n{stderr}"));
    for (i, want) in expected.iter().enumerate() {
        assert_eq!(
            lines.get(start + i).copied().unwrap_or(""),
            want,
            "traceback frame {i} mismatch; full stderr:\n{stderr}"
        );
    }
    assert!(
        lines[start + expected.len()].starts_with("ZeroDivisionError"),
        "the exception line must follow the frames; full stderr:\n{stderr}"
    );
}

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
