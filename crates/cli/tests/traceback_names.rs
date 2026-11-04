//! Traceback frame names and lines match CPython's: a method prints as `m`
//! (not `C.m`), a nested def as `inner` (not `outer.<locals>.inner#0`), a
//! generator as its function name (not `gen.<resume>`), a lambda as `<lambda>`
//! with its own line, and compiler-synthesized value-call thunks contribute no
//! frame. A `for` loop's implicit `next()` is attributed to the `for` line.
//!
//! Checked at `--opt-level none` AND the default level: inlined code carries
//! its inline chain into the traceback table, so the optimizer's inlining
//! must not erase or rename frames. Separately, a line one below the
//! function's first stamped line must not wrap to `line 0`.

use std::path::{Path, PathBuf};
use std::process::Command;

const FIXTURE: &str = r#"import sys
from typing import Callable


class C:
    def m(self, x: int) -> int:
        return 10 // x

    @staticmethod
    def s(x: int) -> int:
        return 10 // x


def outer(x: int) -> int:
    def inner(y: int) -> int:
        return 10 // y
    return inner(x)


def gen(n: int):
    yield 1
    yield 10 // n


def apply(f: Callable[[int], int], v: int) -> int:
    return f(v)


def plain(x: int) -> int:
    return 10 // x


def lam_user(x: int) -> int:
    g = lambda y: 10 // y
    return g(x)


def run(which: int) -> None:
    if which == 0:
        C().m(0)
    elif which == 1:
        outer(0)
    elif which == 2:
        for v in gen(0):
            pass
    elif which == 3:
        apply(plain, 0)
    elif which == 4:
        C.s(0)
    elif which == 5:
        lam_user(0)


run(int(sys.argv[1]))
"#;

/// `(case, [(line, name)…])` — CPython 3.14's frames, outermost first.
const EXPECTED: &[(&str, &[(u32, &str)])] = &[
    ("0", &[(54, "<module>"), (40, "run"), (7, "m")]),
    (
        "1",
        &[(54, "<module>"), (42, "run"), (17, "outer"), (16, "inner")],
    ),
    ("2", &[(54, "<module>"), (44, "run"), (22, "gen")]),
    (
        "3",
        &[(54, "<module>"), (47, "run"), (26, "apply"), (30, "plain")],
    ),
    ("4", &[(54, "<module>"), (49, "run"), (11, "s")]),
    (
        "5",
        &[
            (54, "<module>"),
            (51, "run"),
            (35, "lam_user"),
            (34, "<lambda>"),
        ],
    ),
];

fn compile(source: &Path, exe: &Path, extra: &[&str]) {
    let pyaot = PathBuf::from(env!("CARGO_BIN_EXE_pyaot"));
    let runtime_lib = ensure_runtime_lib(pyaot.parent().expect("target dir"));
    let out = Command::new(&pyaot)
        .arg(source)
        .arg("-o")
        .arg(exe)
        .args(extra)
        .arg("--runtime-lib")
        .arg(&runtime_lib)
        .output()
        .expect("spawn pyaot");
    assert!(
        out.status.success(),
        "pyaot failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pyaot_tbnames_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

#[test]
fn frame_names_match_cpython() {
    let dir = scratch();
    let source = dir.join("tb_names.py");
    std::fs::write(&source, FIXTURE).unwrap();
    let file = source.display().to_string();

    for level in ["none", "speed"] {
        let exe = dir.join(format!("tb_names_{level}"));
        compile(&source, &exe, &["--opt-level", level]);
        for (case, frames) in EXPECTED {
            let run = Command::new(&exe).arg(case).output().expect("run fixture");
            assert!(!run.status.success(), "case {case} must raise");
            let stderr = String::from_utf8_lossy(&run.stderr);
            assert_eq!(
                file_lines(&stderr),
                want_lines(&file, frames),
                "case {case} @ {level}; full stderr:\n{stderr}"
            );
            assert!(
                stderr.contains("ZeroDivisionError"),
                "case {case}: {stderr}"
            );
        }
    }
}

fn file_lines(stderr: &str) -> Vec<&str> {
    stderr
        .lines()
        .filter(|l| l.starts_with("  File "))
        .collect()
}

fn want_lines(file: &str, frames: &[(u32, &str)]) -> Vec<String> {
    frames
        .iter()
        .map(|(line, name)| format!("  File \"{file}\", line {line}, in {name}"))
        .collect()
}

#[test]
fn srcloc_one_below_base_is_not_line_zero() {
    // `boom`'s body (line 2) is inlined into `<module>`, whose first stamped
    // line is 3: the relative srcloc `2 - 3` used to wrap onto Cranelift's
    // "no location" sentinel and print `line 0`. The inlined frame is kept.
    let dir = scratch();
    let source = dir.join("tb_wrap.py");
    std::fs::write(
        &source,
        "def boom(x: int) -> int:\n    return 10 // x\nprint(boom(0))\n",
    )
    .unwrap();
    let exe = dir.join("tb_wrap");
    compile(&source, &exe, &[]);
    let run = Command::new(&exe).output().expect("run fixture");
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(!run.status.success());
    assert!(
        !stderr.contains("line 0,"),
        "no frame may resolve to line 0:\n{stderr}"
    );
    let file = source.display().to_string();
    assert_eq!(
        file_lines(&stderr),
        want_lines(&file, &[(3, "<module>"), (2, "boom")]),
        "{stderr}"
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
