//! Phase-0 benchmark harness for the pyaot compiler.
//!
//! This crate hosts `cargo bench` targets; it exports no public API. The
//! benchmark sources live under `bench/py/` and the harness that drives them
//! is in `bench/benches/pyaot_bench.rs`. See `bench/README.md` for how to
//! run the suite and compare against the committed baseline in
//! `bench/BASELINE.md`.
