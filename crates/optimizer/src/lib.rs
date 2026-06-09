//! # optimizer — passes over typed MIR
//!
//! **Scaffold.** Representation-preserving rewrites over typed MIR: devirtualize,
//! flatten-properties, inline, constfold, peephole, dce, cold-block annotation,
//! plus monomorphization. An `OptimizationPass` trait + a `PassManager` drive them.
//!
//! Hard rule: passes read [`pyaot_types::Repr`] off MIR values — never any
//! inference-internal state. Optimization may not change representation
//! correctness; the verifier runs after every pass.

#![forbid(unsafe_code)]
