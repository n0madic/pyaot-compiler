//! A compile error inside an IMPORTED module is rendered against that module's
//! file (name, line, and source snippet) — not against the entry script. Every
//! module gets a disjoint program-wide span range (`pyaot_utils::SourceMap`),
//! so a bare `Span` identifies its file.

use std::path::PathBuf;
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pyaot_impdiag_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Compile `main.py` in `dir`; return stderr (the compile must fail).
fn compile_err(dir: &std::path::Path) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_pyaot"))
        .arg(dir.join("main.py"))
        .arg("--emit-mir")
        .output()
        .expect("spawn pyaot");
    assert!(!out.status.success(), "the compile must fail");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn name_error_in_imported_module_points_at_that_module() {
    let dir = scratch("name");
    std::fs::write(dir.join("main.py"), "import helper\n\nprint(helper.f(1))\n").unwrap();
    std::fs::write(
        dir.join("helper.py"),
        "# helper module\n\ndef f(x: int) -> int:\n    return x + undefined_name\n",
    )
    .unwrap();
    let stderr = compile_err(&dir);
    let helper = dir.join("helper.py").display().to_string();
    assert!(
        stderr.contains(&format!("{helper}:4:16")),
        "diagnostic must name helper.py line 4:\n{stderr}"
    );
    assert!(
        stderr.contains("return x + undefined_name"),
        "the snippet must come from helper.py:\n{stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn parse_error_in_imported_module_points_at_that_module() {
    let dir = scratch("parse");
    std::fs::write(dir.join("main.py"), "import bad\nprint(1)\n").unwrap();
    std::fs::write(dir.join("bad.py"), "x = 1\ndef g(:\n").unwrap();
    let stderr = compile_err(&dir);
    let bad = dir.join("bad.py").display().to_string();
    assert!(
        stderr.contains(&format!("{bad}:2:7")),
        "diagnostic must name bad.py line 2:\n{stderr}"
    );
    assert!(stderr.contains("def g(:"), "snippet from bad.py:\n{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn entry_script_errors_still_render_against_the_entry() {
    let dir = scratch("entry");
    std::fs::write(dir.join("helper.py"), "def f() -> int:\n    return 1\n").unwrap();
    std::fs::write(
        dir.join("main.py"),
        "import helper\nprint(helper.f())\nprint(nope)\n",
    )
    .unwrap();
    let stderr = compile_err(&dir);
    let main = dir.join("main.py").display().to_string();
    assert!(
        stderr.contains(&format!("{main}:3:7")),
        "diagnostic must name main.py line 3:\n{stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
