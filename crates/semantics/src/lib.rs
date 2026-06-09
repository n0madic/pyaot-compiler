//! # Semantics — name resolution, scopes, MRO
//!
//! **Scaffold.** Resolves names to symbols, builds scope and closure-capture
//! information (a free-variable scan with transitive bubbling so inner closures
//! capture through intermediate scopes), and computes the C3 linearization so
//! multiple inheritance, vtable layout, and `super()` dispatch share one
//! authoritative MRO.

#![forbid(unsafe_code)]
