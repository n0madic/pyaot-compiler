//! `input()` gate: reading a line from stdin (with an optional prompt written to
//! stdout) must match CPython byte-for-byte. This cannot live in the differential
//! corpus — that harness runs the compiled binary with no stdin, so an
//! `input()` program would hit EOF immediately. Here we feed the SAME piped
//! stdin to both the pyaot binary and CPython and compare stdout.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A line read with `input(prompt)` writes `prompt` (no newline) to stdout, then
/// returns the line (trailing newline stripped). The bare `input()` form writes
/// no prompt. The interleaving of prompts and `print` output is the byte-exact
/// behaviour CPython produces when stdin is piped.
const FIXTURE: &str = r#"name = input("Name: ")
print("Hello, " + name)
age = input("Age: ")
print("Age is " + age)
bare = input()
print("Got: " + bare)
"#;

const STDIN: &str = "Alice\n42\nbob\n";

/// EOF on stdin raises `EOFError` (matching CPython), catchable like any builtin
/// exception; the prompt is still written before the read fails.
const EOF_FIXTURE: &str = r#"try:
    x = input("prompt: ")
    print("got " + x)
except EOFError:
    print("EOF caught")
"#;

#[test]
fn input_matches_cpython_with_piped_stdin() {
    let (pyaot, runtime_lib, out_dir) = setup();

    let source = out_dir.join("input_fixture.py");
    std::fs::write(&source, FIXTURE).expect("write fixture");
    let exe = out_dir.join("input_fixture");
    compile(&pyaot, &runtime_lib, &source, &exe);

    let got = run_with_stdin(&exe, STDIN);
    let oracle = python_with_stdin(&source, STDIN);
    assert_eq!(got, oracle, "input() stdout must match CPython");
    assert_eq!(
        got, "Name: Hello, Alice\nAge: Age is 42\nGot: bob\n",
        "input() prompt/line interleaving regressed"
    );
}

#[test]
fn input_eof_raises_eoferror() {
    let (pyaot, runtime_lib, out_dir) = setup();

    let source = out_dir.join("input_eof_fixture.py");
    std::fs::write(&source, EOF_FIXTURE).expect("write fixture");
    let exe = out_dir.join("input_eof_fixture");
    compile(&pyaot, &runtime_lib, &source, &exe);

    let got = run_with_stdin(&exe, "");
    let oracle = python_with_stdin(&source, "");
    assert_eq!(got, oracle, "input() EOF behaviour must match CPython");
    assert_eq!(got, "prompt: EOF caught\n", "EOFError handling regressed");
}

fn setup() -> (PathBuf, PathBuf, PathBuf) {
    let pyaot = PathBuf::from(env!("CARGO_BIN_EXE_pyaot"));
    let target_dir = pyaot.parent().expect("target dir").to_path_buf();
    let runtime_lib = ensure_runtime_lib(&target_dir);
    let out_dir = std::env::temp_dir().join("pyaot_input_test");
    std::fs::create_dir_all(&out_dir).expect("create temp out dir");
    (pyaot, runtime_lib, out_dir)
}

fn compile(pyaot: &Path, runtime_lib: &Path, source: &Path, exe: &Path) {
    let compile = Command::new(pyaot)
        .arg(source)
        .arg("-o")
        .arg(exe)
        .arg("--runtime-lib")
        .arg(runtime_lib)
        .output()
        .expect("failed to spawn pyaot");
    assert!(
        compile.status.success(),
        "pyaot failed to compile the input fixture:\n{}",
        String::from_utf8_lossy(&compile.stderr),
    );
}

fn run_with_stdin(exe: &Path, stdin: &str) -> String {
    let mut child = Command::new(exe)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn compiled fixture");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait for fixture");
    assert!(
        out.status.success(),
        "compiled fixture exited nonzero:\n{}",
        String::from_utf8_lossy(&out.stderr),
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn python_with_stdin(source: &Path, stdin: &str) -> String {
    let mut child = Command::new("python3")
        .arg(source)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn python3");
    child
        .stdin
        .take()
        .expect("python stdin")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait for python3");
    assert!(
        out.status.success(),
        "python3 exited nonzero:\n{}",
        String::from_utf8_lossy(&out.stderr),
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
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
