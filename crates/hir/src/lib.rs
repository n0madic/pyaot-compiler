//! # HIR — High-level IR (CFG-only)
//!
//! **Scaffold.** Control-flow-graph IR: functions own `blocks`, `entry_block`,
//! and `try_scopes`; structured control flow lives in an `HirTerminator`, not in
//! nested statement variants. Generators are desugared into regular functions at
//! this level.
//!
//! Responsibilities:
//! * a unified `BindingTarget` enum — one shape for every binding site
//!   (assignment, `for`, `with`, comprehension clauses), with a `for_each_var`
//!   walker;
//! * `CfgBuilder` / `HirTerminator` / `HirBlock`.
//!
//! Every typed slot carries a [`pyaot_types::SemTy`] only — physical
//! representation ([`pyaot_types::Repr`]) is assigned later at the lowering
//! boundary, never stored here. There is no representation-ambiguous `Any` here.

#![forbid(unsafe_code)]
