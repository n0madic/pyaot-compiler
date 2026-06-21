//! Exception-chaining gate (PEP 3134): an UNHANDLED `raise X from Y` /
//! `from None` / implicit-context exception prints the CPython-format chain
//! (`__cause__` / `__context__`) to stderr and exits nonzero. Like
//! `traceback.rs`, this cannot live in the differential corpus (the gate there
//! requires exit 0 on both sides and compares stdout only), so the expected
//! chain text is asserted directly.
//!
//! Compiled at `--opt-level none` so the chain frames are not collapsed by
//! inlining.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Compile `source` and run it; return its (stdout, stderr, success) triple.
fn compile_and_run(name: &str, source: &str) -> (String, String, bool) {
    let pyaot = PathBuf::from(env!("CARGO_BIN_EXE_pyaot"));
    let target_dir = pyaot.parent().expect("target dir").to_path_buf();
    let runtime_lib = ensure_runtime_lib(&target_dir);

    let out_dir = std::env::temp_dir().join("pyaot_tb_chain_test");
    std::fs::create_dir_all(&out_dir).expect("create temp out dir");
    let src = out_dir.join(format!("{name}.py"));
    std::fs::write(&src, source).expect("write fixture");
    let exe = out_dir.join(name);

    let compile = Command::new(&pyaot)
        .arg(&src)
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
        "pyaot failed to compile {name}:\n{}",
        String::from_utf8_lossy(&compile.stderr),
    );

    let run = Command::new(&exe).output().expect("failed to run fixture");
    (
        String::from_utf8_lossy(&run.stdout).into_owned(),
        String::from_utf8_lossy(&run.stderr).into_owned(),
        run.status.success(),
    )
}

#[test]
fn explicit_cause_prints_direct_cause_chain() {
    let (_out, stderr, ok) = compile_and_run(
        "chain_cause",
        "raise ValueError(\"main\") from TypeError(\"cause\")\n",
    );
    assert!(!ok, "an unhandled exception must exit nonzero");
    assert!(
        stderr.contains("TypeError: cause"),
        "cause line missing; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("The above exception was the direct cause of the following exception:"),
        "direct-cause connector missing; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("ValueError: main"),
        "main exception line missing; stderr:\n{stderr}"
    );
}

#[test]
fn value_cause_prints_direct_cause_chain() {
    // `raise X from <caught variable>` — the value-cause path. The cause's
    // type + message render in the chain.
    let (_out, stderr, ok) = compile_and_run(
        "chain_value_cause",
        "try:\n    raise ValueError(\"orig\")\nexcept ValueError as e:\n    raise RuntimeError(\"wrapper\") from e\n",
    );
    assert!(!ok, "an unhandled exception must exit nonzero");
    assert!(
        stderr.contains("ValueError: orig"),
        "value-cause line missing; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("The above exception was the direct cause of the following exception:"),
        "direct-cause connector missing; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("RuntimeError: wrapper"),
        "main exception line missing; stderr:\n{stderr}"
    );
}

#[test]
fn from_none_suppresses_context() {
    // `raise X from None` inside an except must NOT print the implicit-context
    // connector (the caught exception is suppressed).
    let (_out, stderr, ok) = compile_and_run(
        "chain_suppress",
        "try:\n    raise KeyError(\"inner\")\nexcept KeyError:\n    raise ValueError(\"outer\") from None\n",
    );
    assert!(!ok, "an unhandled exception must exit nonzero");
    assert!(
        stderr.contains("ValueError: outer"),
        "outer exception line missing; stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("During handling of the above exception"),
        "`from None` must suppress the implicit context; stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("KeyError"),
        "the suppressed context must not be printed; stderr:\n{stderr}"
    );
}

#[test]
fn implicit_context_prints_during_handling() {
    // A raise inside an except with no `from` chains the caught exception as
    // the implicit `__context__`.
    let (_out, stderr, ok) = compile_and_run(
        "chain_context",
        "try:\n    raise KeyError(\"inner\")\nexcept KeyError:\n    raise ValueError(\"outer\")\n",
    );
    assert!(!ok, "an unhandled exception must exit nonzero");
    assert!(
        stderr.contains("During handling of the above exception, another exception occurred:"),
        "implicit-context connector missing; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("ValueError: outer"),
        "outer exception line missing; stderr:\n{stderr}"
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
