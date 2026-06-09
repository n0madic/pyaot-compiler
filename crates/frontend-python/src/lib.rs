//! # frontend-python — parse + desugar → HIR
//!
//! **Scaffold.** Parses with `rustpython-parser` and desugars to HIR: generators,
//! comprehensions, `with`, `match` patterns, decorators, walrus, and PEP 563
//! string forward references (with a top-level class pre-scan so forward
//! references resolve regardless of declaration order).
//!
//! Output: HIR annotated with `SemTy` where the source provides it (annotations,
//! literals); everything else is left for `pyaot_typeck` to infer.

#![forbid(unsafe_code)]
