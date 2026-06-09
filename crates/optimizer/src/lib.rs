//! # optimizer — passes over typed MIR
//!
//! Representation-preserving rewrites over typed MIR: devirtualize,
//! flatten-properties, inline, constfold, peephole, dce, cold-block annotation,
//! plus monomorphization. An [`OptimizationPass`] trait + a [`PassManager`] drive
//! them.
//!
//! Hard rule: passes read [`pyaot_types::Repr`] off MIR values — never any
//! inference-internal state. This crate depends on `mir` + `types` but **not**
//! `typeck`; the dependency graph structurally prevents reading inference state
//! (Principle 6). Optimization may not change representation correctness, so the
//! verifier runs at the entry boundary and after every pass in debug builds.
//!
//! ## Phase 1 scope
//!
//! [`PassManager::phase1`] is an **empty** pipeline: it still runs
//! [`pyaot_mir::verify`] at the boundary (in debug), proving the verifier
//! discipline holds from the first MIR ever produced — before any pass exists to
//! perturb it.

#![forbid(unsafe_code)]

use pyaot_mir::{verify, MirProgram, VerifyError};

/// A representation-preserving rewrite over typed MIR. Passes are infallible
/// transformations; the verifier (run by the [`PassManager`] after each pass) is
/// the gate that rejects any pass that breaks representation consistency.
pub trait OptimizationPass {
    fn name(&self) -> &'static str;
    fn run(&self, program: &mut MirProgram);
}

/// Drives a pipeline of [`OptimizationPass`]es, verifying the program at the
/// entry boundary and after each pass.
#[derive(Default)]
pub struct PassManager {
    passes: Vec<Box<dyn OptimizationPass>>,
}

impl PassManager {
    /// The Phase 1 pipeline: no passes. The boundary verify still runs, so the
    /// verifier discipline is exercised end-to-end before any pass exists.
    pub fn phase1() -> Self {
        Self { passes: Vec::new() }
    }

    /// Append a pass (used as the pipeline grows in later phases).
    pub fn push(&mut self, pass: Box<dyn OptimizationPass>) {
        self.passes.push(pass);
    }

    /// Run the pipeline. Verifies at the entry boundary and after every pass
    /// (in debug builds); a verification failure aborts and is returned.
    pub fn run(&self, program: &mut MirProgram) -> Result<(), VerifyError> {
        verify_all(program)?;
        for pass in &self.passes {
            pass.run(program);
            verify_all(program)?;
        }
        Ok(())
    }
}

/// Verify every function — debug builds only; a no-op in release.
#[cfg(debug_assertions)]
fn verify_all(program: &MirProgram) -> Result<(), VerifyError> {
    for func in &program.funcs {
        verify(func, &program.funcs)?;
    }
    Ok(())
}

#[cfg(not(debug_assertions))]
fn verify_all(_program: &MirProgram) -> Result<(), VerifyError> {
    Ok(())
}
