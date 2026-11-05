//! CLI output-path gates: `--run` executes the output by explicit path (never
//! via `$PATH`), the intermediate object never aliases a user file (`foo.o`
//! survives a compile of `foo.py`), and an output that would overwrite the
//! input source is refused.

use std::path::{Path, PathBuf};
use std::process::Command;

const FIXTURE: &str = "print(\"ran\")\n";

/// A fresh scratch directory per test.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pyaot_paths_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn pyaot() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_pyaot"))
}

#[test]
fn run_uses_explicit_path_and_keeps_user_object_file() {
    let runtime_lib = ensure_runtime_lib(pyaot().parent().expect("target dir"));
    let dir = scratch("run");
    std::fs::write(dir.join("ls.py"), FIXTURE).unwrap();
    // A user file named like the old intermediate object must survive.
    std::fs::write(dir.join("ls.o"), b"user data").unwrap();

    // `-o ls` (bare name, same as a `$PATH` binary): `--run` must execute
    // `./ls`, not `/bin/ls`.
    let out = Command::new(pyaot())
        .current_dir(&dir)
        .args(["ls.py", "-o", "ls", "--run", "--runtime-lib"])
        .arg(&runtime_lib)
        .output()
        .expect("spawn pyaot");
    assert!(
        out.status.success(),
        "pyaot --run failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "ran\n");
    assert_eq!(std::fs::read(dir.join("ls.o")).unwrap(), b"user data");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn output_overwriting_input_is_refused() {
    let runtime_lib = ensure_runtime_lib(pyaot().parent().expect("target dir"));
    let dir = scratch("overwrite");
    // An input without a `.py` extension: the default output (extension
    // stripped) IS the input.
    let src = dir.join("script");
    std::fs::write(&src, FIXTURE).unwrap();
    let out = Command::new(pyaot())
        .arg(&src)
        .arg("--runtime-lib")
        .arg(&runtime_lib)
        .output()
        .expect("spawn pyaot");
    assert!(!out.status.success(), "must refuse to overwrite the source");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("would overwrite the input"),
        "unexpected stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(std::fs::read_to_string(&src).unwrap(), FIXTURE);

    // An explicit `-o` naming the input is refused too.
    let py = dir.join("x.py");
    std::fs::write(&py, FIXTURE).unwrap();
    let out = Command::new(pyaot())
        .arg(&py)
        .arg("-o")
        .arg(dir.join(".").join("x.py"))
        .arg("--runtime-lib")
        .arg(&runtime_lib)
        .output()
        .expect("spawn pyaot");
    assert!(!out.status.success(), "must refuse -o == input");
    assert_eq!(std::fs::read_to_string(&py).unwrap(), FIXTURE);
    let _ = std::fs::remove_dir_all(&dir);
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
