//! # pyaot — compiler entry point
//!
//! **Scaffold.** Orchestrates the pipeline:
//!
//! ```text
//! source ─▶ frontend-python ─▶ HIR ─▶ semantics ─▶ typeck ─▶ lowering(+legalize)
//!        ─▶ MIR(verify) ─▶ optimizer(verify) ─▶ codegen-cranelift ─▶ linker ─▶ exe
//! ```
//!
//! Pipeline wiring lands once the crates have content. For now this is a
//! placeholder so the workspace has a runnable binary target.

#![forbid(unsafe_code)]

fn main() {
    eprintln!("pyaot: skeleton — pipeline not yet implemented");
    std::process::exit(1);
}
